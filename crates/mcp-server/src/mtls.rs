//! mTLS auth (ADR-004 §5).
//!
//! TLS termination at `rulake-mcp` itself — we present a server cert
//! and require + verify a client cert against an operator-supplied CA
//! bundle. The client cert's SHA-256 fingerprint flows into the
//! session-binding tuple as the third axis (alongside principal +
//! client_id), so a stolen JWT replayed from a different client cert
//! is rejected.
//!
//! The principal for mTLS is `mtls:<cert-fingerprint-prefix>` —
//! deterministic per client cert, suitable as the audit principal.
//! For deployments that combine mTLS + JWT (mTLS at the edge for
//! transport-level identity, JWT for application-level scopes), the
//! JWT principal wins for capability mapping; mTLS adds the second
//! identity axis for session binding.

use std::sync::Arc;

use rustls_pemfile::{certs, private_key};
use sha2::{Digest, Sha256};
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::rustls::server::WebPkiClientVerifier;
use tokio_rustls::rustls::{RootCertStore, ServerConfig};
use tokio_rustls::TlsAcceptor;

/// Bundle of operator-supplied TLS material.
#[derive(Debug, Clone)]
pub struct MtlsConfig {
    /// PEM-encoded server cert chain (typically `cert + intermediates`).
    pub server_cert_pem: Vec<u8>,
    /// PEM-encoded server private key (PKCS#8 / SEC1).
    pub server_key_pem: Vec<u8>,
    /// PEM-encoded CA bundle that issued the *clients* we'll accept.
    /// Required — without this we have nothing to validate client
    /// certs against.
    pub client_ca_pem: Vec<u8>,
}

/// Build a `TlsAcceptor` that requires + validates client certs.
pub fn build_acceptor(config: &MtlsConfig) -> anyhow::Result<TlsAcceptor> {
    // Server cert chain.
    let mut cert_reader = std::io::BufReader::new(&config.server_cert_pem[..]);
    let server_chain: Vec<CertificateDer<'static>> = certs(&mut cert_reader)
        .collect::<std::io::Result<Vec<_>>>()
        .map_err(|e| anyhow::anyhow!("server cert PEM: {e}"))?;
    if server_chain.is_empty() {
        anyhow::bail!("server cert PEM contained no CERTIFICATE blocks");
    }

    // Server private key.
    let mut key_reader = std::io::BufReader::new(&config.server_key_pem[..]);
    let server_key: PrivateKeyDer<'static> = private_key(&mut key_reader)
        .map_err(|e| anyhow::anyhow!("server key PEM: {e}"))?
        .ok_or_else(|| {
            anyhow::anyhow!("server key PEM contained no recognized PRIVATE KEY block")
        })?;

    // Client CA(s) we'll trust.
    let mut ca_reader = std::io::BufReader::new(&config.client_ca_pem[..]);
    let mut client_root = RootCertStore::empty();
    let mut added = 0usize;
    for cert in certs(&mut ca_reader) {
        let cert = cert.map_err(|e| anyhow::anyhow!("client CA PEM: {e}"))?;
        client_root
            .add(cert)
            .map_err(|e| anyhow::anyhow!("client CA add: {e}"))?;
        added += 1;
    }
    if added == 0 {
        anyhow::bail!("client CA PEM contained no CERTIFICATE blocks");
    }
    let verifier = WebPkiClientVerifier::builder(Arc::new(client_root))
        .build()
        .map_err(|e| anyhow::anyhow!("client verifier build: {e}"))?;

    let server_config = ServerConfig::builder()
        .with_client_cert_verifier(verifier)
        .with_single_cert(server_chain, server_key)
        .map_err(|e| anyhow::anyhow!("rustls server config: {e}"))?;
    Ok(TlsAcceptor::from(Arc::new(server_config)))
}

/// SHA-256 hex of a DER-encoded client certificate.
/// This is the value that flows into the session-binding tuple.
pub fn cert_sha256_hex(der: &[u8]) -> String {
    let digest = Sha256::digest(der);
    hex_lower(&digest)
}

/// SubjectPublicKeyInfo SHA-256 — what RFC 7469 (HPKP) and most
/// pinning schemes use. We expose both forms since v0.7 may
/// surface either as the audit identifier.
#[allow(dead_code)]
pub fn spki_sha256_hex(der: &[u8]) -> String {
    // Best-effort SPKI extraction would need x509-parser. v0.6 ships
    // the full-cert digest only; SPKI digest lands in v0.7 alongside
    // the x509-parser dep.
    cert_sha256_hex(der)
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

/// Build the mTLS principal string from a client cert fingerprint —
/// `mtls:<first-16-hex>`. Short prefix keeps the audit log readable
/// while staying collision-resistant for any realistic deployment.
pub fn principal_for_client_cert(der: &[u8]) -> String {
    let full = cert_sha256_hex(der);
    let prefix = &full[..16.min(full.len())];
    format!("mtls:{prefix}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cert_fingerprint_is_64_hex() {
        let h = cert_sha256_hex(&[0u8, 1, 2, 3, 4]);
        assert_eq!(h.len(), 64);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn cert_fingerprint_is_deterministic() {
        let a = cert_sha256_hex(b"test-cert-bytes");
        let b = cert_sha256_hex(b"test-cert-bytes");
        assert_eq!(a, b);
    }

    #[test]
    fn cert_fingerprint_changes_with_input() {
        let a = cert_sha256_hex(b"client-A");
        let b = cert_sha256_hex(b"client-B");
        assert_ne!(a, b);
    }

    #[test]
    fn principal_has_mtls_prefix() {
        let p = principal_for_client_cert(b"any-cert");
        assert!(p.starts_with("mtls:"));
        // mtls: + 16 hex chars
        assert_eq!(p.len(), 5 + 16);
    }

    #[test]
    fn build_acceptor_rejects_missing_certs() {
        let config = MtlsConfig {
            server_cert_pem: b"".to_vec(),
            server_key_pem: b"".to_vec(),
            client_ca_pem: b"".to_vec(),
        };
        assert!(build_acceptor(&config).is_err());
    }
}
