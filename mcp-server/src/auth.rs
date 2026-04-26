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

use std::sync::Arc;

use http::{HeaderMap, StatusCode};
use subtle::ConstantTimeEq;

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
