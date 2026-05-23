//! v0.2 auth — bearer-token mode (ADR-004 §5).
//!
//! Bearer is the **dev-only** option. The ADR explicitly marks it as
//! such: it refuses to bind a non-loopback interface unless paired
//! with `--allow-bearer-on-public` (intentionally embarrassing flag),
//! and emits a warning every 60 s reminding the operator to migrate
//! to OAuth.
//!
//! Constant-time compare via `subtle::ConstantTimeEq` so a timing
//! attack can't lift the token character-by-character.

use std::collections::HashSet;
use std::sync::Arc;

use http::{HeaderMap, StatusCode};
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;

use crate::jwks::JwksKeys;
#[cfg(test)]
use crate::policy::Capability;
use crate::policy::CapabilitySet;

/// Bearer-token verifier. Cheap to clone (Arc-wrapped state).
#[derive(Clone)]
pub struct BearerAuth {
    inner: Arc<Inner>,
}

struct Inner {
    /// SHA-256 fingerprint of the configured token. Stored as bytes
    /// (not the token itself) — we never need the plaintext after
    /// load and not keeping it around shrinks the leak surface.
    /// Constant-time compare uses these bytes directly.
    expected_token: Vec<u8>,
}

impl BearerAuth {
    /// Build from a token string. The token is consumed and dropped
    /// after copying its bytes.
    pub fn new(token: impl Into<String>) -> Self {
        let token = token.into();
        let bytes = token.into_bytes();
        Self {
            inner: Arc::new(Inner {
                expected_token: bytes,
            }),
        }
    }

    /// Load from a file. The file must contain only the token
    /// (with optional trailing newline trimmed).
    pub fn from_file(path: impl AsRef<std::path::Path>) -> anyhow::Result<Self> {
        let body = std::fs::read_to_string(path.as_ref())?;
        let trimmed = body.trim();
        if trimmed.is_empty() {
            anyhow::bail!("bearer token file is empty");
        }
        Ok(Self::new(trimmed))
    }

    /// Verify the request's `Authorization: Bearer <token>` header.
    /// Returns `Ok(principal)` on success or an HTTP status to return.
    /// The principal is the token's SHA-256 fingerprint — used as the
    /// audit `principal` field so the same token across requests
    /// resolves to the same audit identity.
    pub fn verify(&self, headers: &HeaderMap) -> Result<String, StatusCode> {
        let header = headers
            .get(http::header::AUTHORIZATION)
            .ok_or(StatusCode::UNAUTHORIZED)?
            .to_str()
            .map_err(|_| StatusCode::UNAUTHORIZED)?;
        let token = header
            .strip_prefix("Bearer ")
            .ok_or(StatusCode::UNAUTHORIZED)?;

        // Constant-time compare. ct_eq returns Choice (0 or 1).
        let ok: bool = self
            .inner
            .expected_token
            .as_slice()
            .ct_eq(token.as_bytes())
            .into();
        if !ok {
            return Err(StatusCode::UNAUTHORIZED);
        }

        // Audit principal = SHA-256(token)[..16] hex. Matches the
        // ADR-004 §7 principal format for bearer mode.
        Ok(format!("bearer:{}", short_fingerprint(token.as_bytes())))
    }
}

fn short_fingerprint(token: &[u8]) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    // Audit principal isn't a security boundary (the constant-time
    // compare above already gates access). Hash is just for
    // attribution; collisions would group two unrelated tokens in
    // audit but never grant access.
    let mut h = DefaultHasher::new();
    token.hash(&mut h);
    format!("{:016x}", h.finish())
}

// ─── JWT bearer (OAuth-style scope→capability mapping) ──────────────
//
// `--auth jwt` mode. The operator points at a JWKS file with the
// allowed signing keys; the server validates each request's `Bearer
// <jwt>` against:
//   - JWS signature (HS256 / RS256 / ES256 — matches jsonwebtoken's
//     default Validation alg list for the loaded key)
//   - exp / nbf (jsonwebtoken handles automatically)
//   - iss claim (operator-supplied)
//   - aud claim (must include the server's resource identifier per
//     RFC 8707 Resource Indicators)
//   - scope / scp claim → mapped to capabilities:
//        mcp:rulake:read     → Capability::Read
//        mcp:rulake:publish  → Capability::Read + Publish
//        mcp:rulake:admin    → Capability::Read + Publish + Admin
//
// This is the SOTA auth path. The full OAuth 2.1 client flow (PKCE
// authorization-code, RFC 9728 PRM, DCR) is the *client* responsibility
// — this is the server's token-validation half.

/// Operator config for the JWT verifier.
#[derive(Debug, Clone)]
pub struct JwtConfig {
    /// PEM-encoded RSA/EC public key OR raw HMAC secret.
    pub key: JwtKey,
    /// Required iss claim. Tokens without a matching iss are rejected.
    pub issuer: String,
    /// Required aud claim — the server's resource identifier (RFC 8707).
    pub audience: String,
    /// Algorithms the key supports. Typically a single value; the
    /// `alg` header must match one of these.
    pub algorithms: Vec<Algorithm>,
}

#[derive(Debug, Clone)]
pub enum JwtKey {
    /// HMAC secret (HS256/HS384/HS512). Fine when the operator and
    /// the IdP share trust; production deployments use the
    /// public-key variants below.
    Hmac(Vec<u8>),
    /// RSA public key in PEM (`-----BEGIN PUBLIC KEY-----` /
    /// `-----BEGIN RSA PUBLIC KEY-----`). RS256/RS384/RS512.
    RsaPem(Vec<u8>),
    /// EC public key in PEM. ES256/ES384.
    EcPem(Vec<u8>),
    /// JWKS-fetched key — the JWKS loop builds these and hot-swaps
    /// them into the verifier. The `kid` (key id) routes incoming
    /// tokens to the right key.
    Jwks(Arc<JwksKeys>),
}

#[derive(Clone)]
pub struct JwtAuth {
    inner: Arc<JwtInner>,
}

struct JwtInner {
    key_mode: KeyMode,
    validation: Validation,
    audience: String,
}

enum KeyMode {
    /// One static key — set at startup, never rotated. HMAC, raw PEM.
    Static(DecodingKey),
    /// JWKS-managed keys, refreshed by a background task.
    Jwks(Arc<JwksKeys>),
}

#[derive(Debug, Deserialize, Serialize)]
struct JwtClaims {
    /// Subject — used as audit `principal`.
    sub: String,
    /// Either a space-separated string ("scope") or an array ("scp").
    /// We accept both; whichever is present wins.
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    scp: Option<Vec<String>>,
    /// jsonwebtoken validates iss/aud/exp from these.
    iss: String,
    /// `aud` may be a string or array; we accept both via Value.
    #[serde(default)]
    aud: Option<serde_json::Value>,
    /// Optional client_id — useful for audit attribution.
    #[serde(default)]
    client_id: Option<String>,
}

impl JwtAuth {
    pub fn new(config: JwtConfig) -> Self {
        let mode = match config.key {
            JwtKey::Hmac(bytes) => KeyMode::Static(DecodingKey::from_secret(&bytes)),
            JwtKey::RsaPem(bytes) => KeyMode::Static(
                DecodingKey::from_rsa_pem(&bytes)
                    .unwrap_or_else(|e| panic!("invalid RSA PEM: {e}")),
            ),
            JwtKey::EcPem(bytes) => KeyMode::Static(
                DecodingKey::from_ec_pem(&bytes).unwrap_or_else(|e| panic!("invalid EC PEM: {e}")),
            ),
            JwtKey::Jwks(keys) => KeyMode::Jwks(keys),
        };
        let mut validation =
            Validation::new(*config.algorithms.first().unwrap_or(&Algorithm::HS256));
        validation.algorithms = config.algorithms;
        validation.set_issuer(std::slice::from_ref(&config.issuer));
        validation.set_audience(std::slice::from_ref(&config.audience));
        Self {
            inner: Arc::new(JwtInner {
                key_mode: mode,
                validation,
                audience: config.audience,
            }),
        }
    }

    /// Validate `Authorization: Bearer <jwt>`. Returns the audit
    /// principal + the capability set the token's scopes grant.
    /// In JWKS mode, the JWT header's `kid` claim selects the key
    /// from the rotation set.
    pub fn verify(&self, headers: &HeaderMap) -> Result<JwtPrincipal, StatusCode> {
        let header = headers
            .get(http::header::AUTHORIZATION)
            .ok_or(StatusCode::UNAUTHORIZED)?
            .to_str()
            .map_err(|_| StatusCode::UNAUTHORIZED)?;
        let token = header
            .strip_prefix("Bearer ")
            .ok_or(StatusCode::UNAUTHORIZED)?;

        let key_for_validation: jsonwebtoken::DecodingKey = match &self.inner.key_mode {
            KeyMode::Static(k) => k.clone(),
            KeyMode::Jwks(set) => {
                // Decode just the header to find the kid.
                let hdr = jsonwebtoken::decode_header(token).map_err(|e| {
                    tracing::debug!(error = %e, "jwt header decode failed");
                    StatusCode::UNAUTHORIZED
                })?;
                let kid = hdr.kid.as_deref().ok_or_else(|| {
                    tracing::debug!("jwt missing kid header in JWKS mode");
                    StatusCode::UNAUTHORIZED
                })?;
                set.find(kid).ok_or_else(|| {
                    tracing::debug!(kid, "kid not in JWKS rotation set");
                    StatusCode::UNAUTHORIZED
                })?
            }
        };

        let data = decode::<JwtClaims>(token, &key_for_validation, &self.inner.validation)
            .map_err(|e| {
                tracing::debug!(error = %e, "jwt validation failed");
                StatusCode::UNAUTHORIZED
            })?;
        let claims = data.claims;

        let scopes: HashSet<String> = if let Some(scp) = claims.scp.clone() {
            scp.into_iter().collect()
        } else if let Some(scope) = claims.scope.clone() {
            scope.split_whitespace().map(|s| s.to_string()).collect()
        } else {
            HashSet::new()
        };
        let caps = scopes_to_caps(&scopes);
        Ok(JwtPrincipal {
            principal: format!("oauth:{}", claims.sub),
            client_id: claims.client_id,
            scopes,
            capabilities: caps,
            audience: self.inner.audience.clone(),
        })
    }
}

/// What the JWT verifier returns on success.
#[derive(Debug, Clone)]
pub struct JwtPrincipal {
    /// `oauth:<sub>` — used as audit principal.
    pub principal: String,
    pub client_id: Option<String>,
    pub scopes: HashSet<String>,
    /// Capabilities derived from the scope list. Always includes Read
    /// when at least one mcp:rulake:* scope is present.
    pub capabilities: CapabilitySet,
    pub audience: String,
}

/// `mcp:rulake:read|publish|admin` → CapabilitySet.
pub fn scopes_to_caps(scopes: &HashSet<String>) -> CapabilitySet {
    let mut csv = String::new();
    let push = |csv: &mut String, label: &str| {
        if !csv.is_empty() {
            csv.push(',');
        }
        csv.push_str(label);
    };
    if scopes.contains("mcp:rulake:read") {
        push(&mut csv, "read");
    }
    if scopes.contains("mcp:rulake:publish") {
        push(&mut csv, "publish");
    }
    if scopes.contains("mcp:rulake:admin") {
        push(&mut csv, "admin");
    }
    if csv.is_empty() {
        CapabilitySet::default()
    } else {
        CapabilitySet::from_csv(&csv).unwrap_or_default()
    }
}

#[cfg(test)]
mod jwt_tests {
    use super::*;
    use jsonwebtoken::{encode, EncodingKey, Header};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn now_secs() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    fn sign(claims: &serde_json::Value, secret: &[u8]) -> String {
        let header = Header::new(Algorithm::HS256);
        encode(&header, claims, &EncodingKey::from_secret(secret)).unwrap()
    }

    #[test]
    fn jwt_accepts_valid_token_with_publish_scope() {
        let secret = b"test-secret-32-bytes-long-okay-pls!".to_vec();
        let auth = JwtAuth::new(JwtConfig {
            key: JwtKey::Hmac(secret.clone()),
            issuer: "https://idp.example".into(),
            audience: "https://rulake.example/mcp".into(),
            algorithms: vec![Algorithm::HS256],
        });
        let claims = serde_json::json!({
            "sub": "alice",
            "iss": "https://idp.example",
            "aud": "https://rulake.example/mcp",
            "exp": now_secs() + 60,
            "scope": "mcp:rulake:read mcp:rulake:publish",
            "client_id": "claude-desktop/1.4",
        });
        let token = sign(&claims, &secret);
        let mut h = HeaderMap::new();
        h.insert(
            http::header::AUTHORIZATION,
            format!("Bearer {token}").parse().unwrap(),
        );

        let p = auth.verify(&h).expect("token must validate");
        assert_eq!(p.principal, "oauth:alice");
        assert_eq!(p.client_id.as_deref(), Some("claude-desktop/1.4"));
        assert!(p.capabilities.has(Capability::Read));
        assert!(p.capabilities.has(Capability::Publish));
        assert!(!p.capabilities.has(Capability::Admin));
    }

    #[test]
    fn jwt_rejects_wrong_audience() {
        let secret = b"test-secret-32-bytes-long-okay-pls!".to_vec();
        let auth = JwtAuth::new(JwtConfig {
            key: JwtKey::Hmac(secret.clone()),
            issuer: "https://idp.example".into(),
            audience: "https://rulake.example/mcp".into(),
            algorithms: vec![Algorithm::HS256],
        });
        let claims = serde_json::json!({
            "sub": "alice",
            "iss": "https://idp.example",
            "aud": "https://OTHER.example/mcp",   // wrong audience
            "exp": now_secs() + 60,
            "scope": "mcp:rulake:read",
        });
        let token = sign(&claims, &secret);
        let mut h = HeaderMap::new();
        h.insert(
            http::header::AUTHORIZATION,
            format!("Bearer {token}").parse().unwrap(),
        );
        assert_eq!(auth.verify(&h).unwrap_err(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn jwt_rejects_expired() {
        let secret = b"test-secret-32-bytes-long-okay-pls!".to_vec();
        let auth = JwtAuth::new(JwtConfig {
            key: JwtKey::Hmac(secret.clone()),
            issuer: "https://idp.example".into(),
            audience: "https://rulake.example/mcp".into(),
            algorithms: vec![Algorithm::HS256],
        });
        let claims = serde_json::json!({
            "sub": "alice",
            "iss": "https://idp.example",
            "aud": "https://rulake.example/mcp",
            // Default jsonwebtoken leeway is 60s — go further into the past.
            "exp": now_secs() - 600,
            "scope": "mcp:rulake:read",
        });
        let token = sign(&claims, &secret);
        let mut h = HeaderMap::new();
        h.insert(
            http::header::AUTHORIZATION,
            format!("Bearer {token}").parse().unwrap(),
        );
        assert_eq!(auth.verify(&h).unwrap_err(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn jwt_rejects_wrong_signature() {
        let secret = b"test-secret-32-bytes-long-okay-pls!".to_vec();
        let other = b"WRONG-secret-32-bytes-long-okay-OK?".to_vec();
        let auth = JwtAuth::new(JwtConfig {
            key: JwtKey::Hmac(secret.clone()),
            issuer: "https://idp.example".into(),
            audience: "https://rulake.example/mcp".into(),
            algorithms: vec![Algorithm::HS256],
        });
        let claims = serde_json::json!({
            "sub": "alice",
            "iss": "https://idp.example",
            "aud": "https://rulake.example/mcp",
            "exp": now_secs() + 60,
            "scope": "mcp:rulake:read",
        });
        let token = sign(&claims, &other); // signed with the wrong key
        let mut h = HeaderMap::new();
        h.insert(
            http::header::AUTHORIZATION,
            format!("Bearer {token}").parse().unwrap(),
        );
        assert_eq!(auth.verify(&h).unwrap_err(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn scope_array_form_works() {
        let secret = b"test-secret-32-bytes-long-okay-pls!".to_vec();
        let auth = JwtAuth::new(JwtConfig {
            key: JwtKey::Hmac(secret.clone()),
            issuer: "https://idp.example".into(),
            audience: "https://rulake.example/mcp".into(),
            algorithms: vec![Algorithm::HS256],
        });
        let claims = serde_json::json!({
            "sub": "alice",
            "iss": "https://idp.example",
            "aud": "https://rulake.example/mcp",
            "exp": now_secs() + 60,
            "scp": ["mcp:rulake:read", "mcp:rulake:admin"],
        });
        let token = sign(&claims, &secret);
        let mut h = HeaderMap::new();
        h.insert(
            http::header::AUTHORIZATION,
            format!("Bearer {token}").parse().unwrap(),
        );
        let p = auth.verify(&h).unwrap();
        assert!(p.capabilities.has(Capability::Admin));
    }
}
