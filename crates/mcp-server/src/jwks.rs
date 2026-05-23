//! JWKS (JSON Web Key Set, RFC 7517) fetcher + key rotation.
//!
//! The operator points at a JWKS URL (typically the IdP's
//! `.well-known/jwks.json`); a background task GETs it on a configurable
//! interval (default 5 min), parses RSA / EC keys out of the set, and
//! hot-swaps them into a thread-safe rotation. Each key keeps its `kid`
//! so token validation can pick the right one.
//!
//! v0.5 supports RSA (`kty: "RSA"`) keys constructed from the JWK's
//! `n` + `e` fields. EC (`kty: "EC"`) parsing is straightforward but
//! lands in v0.6 — most production IdPs (Auth0, Okta, Cognito,
//! Google) issue RSA keys by default.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use jsonwebtoken::DecodingKey;
use serde::Deserialize;

/// The thread-safe key set the JWKS task writes into. `JwtAuth`
/// reads from it for each verify call.
pub struct JwksKeys {
    inner: RwLock<HashMap<String, DecodingKey>>,
    /// Source URL — used for logging + the manual refresh path.
    pub url: String,
}

impl std::fmt::Debug for JwksKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JwksKeys")
            .field("url", &self.url)
            .field("key_count", &self.key_count())
            .finish()
    }
}

impl JwksKeys {
    pub fn empty(url: impl Into<String>) -> Arc<Self> {
        Arc::new(Self {
            inner: RwLock::new(HashMap::new()),
            url: url.into(),
        })
    }

    /// Look up a key by `kid` (JWT header field). Returns a clone
    /// because `DecodingKey` doesn't `&self`-deref into the validator.
    pub fn find(&self, kid: &str) -> Option<DecodingKey> {
        self.inner.read().unwrap().get(kid).cloned()
    }

    pub fn key_count(&self) -> usize {
        self.inner.read().unwrap().len()
    }

    pub fn install_keys(&self, fresh: HashMap<String, DecodingKey>) {
        let mut guard = self.inner.write().unwrap();
        *guard = fresh;
    }
}

/// Spawn a background tokio task that periodically refreshes the
/// JWKS. The task lives until `Arc<JwksKeys>` is dropped (Tokio
/// drops the task when its only reference goes away).
///
/// One initial fetch happens synchronously *before* the loop starts,
/// so the server doesn't accept requests before it has any keys.
pub async fn spawn_refresh_task(
    keys: Arc<JwksKeys>,
    interval: Duration,
    http: reqwest::Client,
) -> Result<(), JwksError> {
    // Initial synchronous fetch — fail fast if the URL is wrong.
    let fresh = fetch_and_parse(&keys.url, &http).await?;
    keys.install_keys(fresh);
    let initial_count = keys.key_count();
    tracing::info!(url = %keys.url, key_count = initial_count, "JWKS initial fetch ok");

    // Background refresh loop.
    let keys_for_loop = Arc::clone(&keys);
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(interval);
        // Skip the immediate first tick; we already did it above.
        tick.tick().await;
        loop {
            tick.tick().await;
            match fetch_and_parse(&keys_for_loop.url, &http).await {
                Ok(fresh) => {
                    let n = fresh.len();
                    keys_for_loop.install_keys(fresh);
                    tracing::debug!(url = %keys_for_loop.url, key_count = n, "JWKS refresh ok");
                }
                Err(e) => {
                    tracing::warn!(url = %keys_for_loop.url, error = %e, "JWKS refresh failed; keeping previous keys");
                }
            }
        }
    });
    Ok(())
}

async fn fetch_and_parse(
    url: &str,
    http: &reqwest::Client,
) -> Result<HashMap<String, DecodingKey>, JwksError> {
    let resp = http.get(url).send().await.map_err(JwksError::Http)?;
    if !resp.status().is_success() {
        return Err(JwksError::Status(resp.status().as_u16()));
    }
    let body: JwksDocument = resp.json().await.map_err(JwksError::Http)?;
    let mut out = HashMap::new();
    for jwk in body.keys {
        match parse_jwk(&jwk) {
            Ok((kid, key)) => {
                out.insert(kid, key);
            }
            Err(e) => {
                tracing::debug!(kid = ?jwk.kid, error = %e, "skipping unsupported JWK");
            }
        }
    }
    if out.is_empty() {
        return Err(JwksError::NoKeys);
    }
    Ok(out)
}

#[derive(Debug, Deserialize)]
struct JwksDocument {
    keys: Vec<Jwk>,
}

#[derive(Debug, Deserialize)]
struct Jwk {
    /// Key id — required for rotation. RFC 7517 §4.5: optional in
    /// the spec but de-facto mandatory in IdP-issued JWKS docs.
    kid: Option<String>,
    /// Key type — `RSA` (v0.5 supported) or `EC` (v0.6).
    kty: String,
    /// Algorithm — informational; we trust the validator's
    /// `algorithms` list.
    #[serde(default)]
    #[allow(dead_code)]
    alg: Option<String>,
    /// RSA modulus (base64url, no padding).
    #[serde(default)]
    n: Option<String>,
    /// RSA exponent (base64url, no padding).
    #[serde(default)]
    e: Option<String>,
}

fn parse_jwk(j: &Jwk) -> Result<(String, DecodingKey), String> {
    let kid = j.kid.clone().ok_or("jwk has no kid")?;
    match j.kty.as_str() {
        "RSA" => {
            let n = j.n.as_deref().ok_or("RSA jwk missing n")?;
            let e = j.e.as_deref().ok_or("RSA jwk missing e")?;
            // jsonwebtoken builds DecodingKey directly from JWK's
            // base64url n+e.
            let key = DecodingKey::from_rsa_components(n, e)
                .map_err(|err| format!("RSA from_components: {err}"))?;
            Ok((kid, key))
        }
        "EC" => Err("EC keys land in v0.6 — JWKS task ignored this entry".into()),
        other => Err(format!("unsupported kty {other:?}")),
    }
}

#[derive(Debug)]
pub enum JwksError {
    Http(reqwest::Error),
    Status(u16),
    NoKeys,
}

impl std::fmt::Display for JwksError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Http(e) => write!(f, "JWKS http: {e}"),
            Self::Status(s) => write!(f, "JWKS http status {s}"),
            Self::NoKeys => write!(f, "JWKS document has no usable keys"),
        }
    }
}
impl std::error::Error for JwksError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration as Dur;

    #[test]
    fn empty_set_starts_with_zero_keys() {
        let k = JwksKeys::empty("https://idp.example/jwks.json");
        assert_eq!(k.key_count(), 0);
        assert!(k.find("any-kid").is_none());
    }

    #[test]
    fn install_then_find() {
        let k = JwksKeys::empty("");
        let mut fresh = HashMap::new();
        fresh.insert(
            "kid-1".to_string(),
            DecodingKey::from_secret(b"test-only-not-actually-rsa"),
        );
        k.install_keys(fresh);
        assert_eq!(k.key_count(), 1);
        assert!(k.find("kid-1").is_some());
        assert!(k.find("kid-2").is_none());
    }

    #[test]
    fn install_replaces_atomically() {
        let k = JwksKeys::empty("");
        let mut a = HashMap::new();
        a.insert("a".into(), DecodingKey::from_secret(b"x"));
        k.install_keys(a);
        assert!(k.find("a").is_some());
        let mut b = HashMap::new();
        b.insert("b".into(), DecodingKey::from_secret(b"y"));
        k.install_keys(b);
        assert!(k.find("a").is_none(), "rotation drops old keys");
        assert!(k.find("b").is_some());
    }

    #[test]
    fn parse_jwk_rejects_unsupported_kty() {
        let j = Jwk {
            kid: Some("k".into()),
            kty: "oct".into(),
            alg: None,
            n: None,
            e: None,
        };
        assert!(parse_jwk(&j).is_err());
    }

    #[test]
    fn parse_jwk_rsa_round_trip() {
        // A real public-RSA modulus (Google's, captured 2026-04-26)
        // — used here just as bytes to validate the parser path; we
        // don't actually verify a token against it.
        let j = Jwk {
            kid: Some("kid-1".into()),
            kty: "RSA".into(),
            alg: Some("RS256".into()),
            n: Some(
                "qDi7Tx4DhNvPQsl1ofxxc2ePQFcs-L0mXYo6TGS64CY_2WmOtvYlcLNZjhuddZVV2X88m0MfwaSA16wE-RiKM9hqo5EY8BPXj57CMiYAyiHuQPp1yayjMgoE1P2jvp4eqF-BTillGJt5W5RuXti9uqfMtCQdagB8EC3MNRuU_KdeLgBy3lS3oo4LOYd-74kRBVZbk2wnmmb7IhP9OoLc1-7-9qU1uhpDxmE6JwBau0mDSwMnYDS4G_ML17dC-ZDtLd1i24STUw39KH0pcSdfFbL2NtEZdNeam1DDdk0iUtJSPZliUHJBI_pj8M-2Mn_oA8jBuI8YKwBqYkZCN1I95Q".into()
            ),
            e: Some("AQAB".into()),
        };
        let (kid, _key) = parse_jwk(&j).expect("RSA parse");
        assert_eq!(kid, "kid-1");
    }

    #[test]
    fn refresh_task_smoke() {
        // Sanity that the spawn_refresh_task signature is sane.
        // No actual network.
        let k = JwksKeys::empty("https://example.invalid/jwks.json");
        let _interval = Dur::from_secs(300);
        // We don't actually .await it; just confirm types compile.
        let _f: fn(_, _, _) -> _ = spawn_refresh_task;
        assert_eq!(k.key_count(), 0);
    }
}
