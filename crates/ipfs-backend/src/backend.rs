//! `IpfsBackend` — `BackendAdapter` over IPFS for bundle distribution
//! (ADR-005). Talks to kubo via plain HTTP RPC.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use serde::Deserialize;

use rulake::backend::{BackendAdapter, CollectionId, PulledBatch};
use rulake::bundle::{Generation, RuLakeBundle};
use rulake::error::{Result, RuLakeError};

// ─── Config types ────────────────────────────────────────────────────

/// kubo HTTP RPC endpoint (default `http://127.0.0.1:5001`).
#[derive(Debug, Clone)]
pub struct KuboApiUrl(pub String);

impl Default for KuboApiUrl {
    fn default() -> Self {
        Self("http://127.0.0.1:5001".into())
    }
}

/// Public IPFS gateway URL — `https://ipfs.io`, `https://dweb.link`,
/// `https://w3s.link`. **Cloudflare's gateway is decommissioned** per
/// ADR-005 §2.
#[derive(Debug, Clone)]
pub struct GatewayUrl(pub String);

impl Default for GatewayUrl {
    /// `ipfs.io` — official IPFS Foundation gateway.
    fn default() -> Self {
        Self("https://ipfs.io".into())
    }
}

/// Operating mode (ADR-005 §2).
#[derive(Debug, Clone)]
pub enum Mode {
    /// Talks to a kubo RPC. Read + write + pin. The default.
    Kubo {
        api: KuboApiUrl,
        /// Optional bearer token — kubo's `API.Authorizations` /
        /// `HTTPAuthSecrets` mechanism (ADR-005 §10).
        api_token: Option<String>,
    },
    /// Read-only via a public gateway. CI / sandbox / no-egress
    /// deployments. `publish_bundle` errors with `RuLakeError::Backend`
    /// when invoked in this mode.
    Gateway { gateway: GatewayUrl },
    /// kubo first; fall back to public gateway on read miss.
    /// Audit-loud per ADR-005 §2 — every fallback emits a WARN.
    KuboWithGatewayFallback {
        api: KuboApiUrl,
        api_token: Option<String>,
        gateway: GatewayUrl,
    },
}

impl Default for Mode {
    fn default() -> Self {
        Self::Kubo {
            api: KuboApiUrl::default(),
            api_token: None,
        }
    }
}

/// One registered (collection, CID) mapping.
#[derive(Debug, Clone)]
pub struct IpfsCollectionConfig {
    pub name: String,
    /// Optional pre-set CID. Operator-supplied if the bundle was
    /// pinned out-of-band; left `None` for the publish-then-read
    /// in-process flow.
    pub cid: Option<String>,
    /// Operator-supplied dim. Lets `current_bundle()` synthesize a
    /// bundle without parsing the on-IPFS body.
    pub dim: Option<usize>,
}

pub type IpfsCollection = IpfsCollectionConfig;

// ─── Backend ─────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct IpfsBackend {
    id: String,
    inner: Arc<Inner>,
}

struct Inner {
    mode: Mode,
    collections: RwLock<HashMap<CollectionId, IpfsCollectionConfig>>,
    /// Single-thread tokio runtime hosting reqwest calls. Owned by
    /// the backend so the sync `BackendAdapter` trait stays satisfied.
    runtime: Arc<tokio::runtime::Runtime>,
    http: reqwest::Client,
}

impl IpfsBackend {
    pub fn new(id: impl Into<String>, mode: Mode) -> Result<Self> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| RuLakeError::InvalidParameter(format!("tokio rt: {e}")))?;
        Ok(Self {
            id: id.into(),
            inner: Arc::new(Inner {
                mode,
                collections: RwLock::new(HashMap::new()),
                runtime: Arc::new(runtime),
                http: reqwest::Client::builder()
                    .build()
                    .map_err(|e| RuLakeError::InvalidParameter(format!("reqwest: {e}")))?,
            }),
        })
    }

    /// kubo at default URL (127.0.0.1:5001), no auth.
    pub fn kubo_local(id: impl Into<String>) -> Result<Self> {
        Self::new(
            id,
            Mode::Kubo {
                api: KuboApiUrl::default(),
                api_token: None,
            },
        )
    }

    /// Gateway-only (read path; CI / sandbox).
    pub fn gateway_only(id: impl Into<String>, gateway: impl Into<String>) -> Result<Self> {
        Self::new(
            id,
            Mode::Gateway {
                gateway: GatewayUrl(gateway.into()),
            },
        )
    }

    pub fn register(&self, c: IpfsCollectionConfig) -> Result<()> {
        if let Some(cid) = &c.cid {
            validate_cid_shape(cid)?;
        }
        self.inner
            .collections
            .write()
            .unwrap()
            .insert(c.name.clone(), c);
        Ok(())
    }

    fn collection(&self, name: &str) -> Result<IpfsCollectionConfig> {
        self.inner
            .collections
            .read()
            .unwrap()
            .get(name)
            .cloned()
            .ok_or_else(|| RuLakeError::UnknownCollection {
                backend: self.id.clone(),
                collection: name.to_string(),
            })
    }

    fn record_cid(&self, collection: &str, cid: String, dim: Option<usize>) {
        let mut map = self.inner.collections.write().unwrap();
        if let Some(c) = map.get_mut(collection) {
            c.cid = Some(cid);
            if dim.is_some() {
                c.dim = dim;
            }
        } else {
            map.insert(
                collection.to_string(),
                IpfsCollectionConfig {
                    name: collection.to_string(),
                    cid: Some(cid),
                    dim,
                },
            );
        }
    }

    fn kubo_url(&self) -> Option<&str> {
        match &self.inner.mode {
            Mode::Kubo { api, .. } | Mode::KuboWithGatewayFallback { api, .. } => Some(&api.0),
            _ => None,
        }
    }

    fn kubo_token(&self) -> Option<&str> {
        match &self.inner.mode {
            Mode::Kubo { api_token, .. } | Mode::KuboWithGatewayFallback { api_token, .. } => {
                api_token.as_deref()
            }
            _ => None,
        }
    }

    fn gateway_url(&self) -> Option<&str> {
        match &self.inner.mode {
            Mode::Gateway { gateway } | Mode::KuboWithGatewayFallback { gateway, .. } => {
                Some(&gateway.0)
            }
            _ => None,
        }
    }

    /// Fetch a CID's bytes — kubo first when available, gateway
    /// fallback when configured. Returns the body as a `Vec<u8>`;
    /// bundles are ≤ 64 KiB by `RuLakeBundle::from_json`'s cap so
    /// unbounded growth isn't a concern.
    pub fn cat(&self, cid_str: &str) -> Result<Vec<u8>> {
        let backend = self.clone();
        let cid_owned = cid_str.to_string();
        self.inner.runtime.block_on(async move {
            // Kubo path first when configured.
            if backend.kubo_url().is_some() {
                match kubo_cat(&backend, &cid_owned).await {
                    Ok(bytes) => return Ok(bytes),
                    Err(e) => match &backend.inner.mode {
                        Mode::KuboWithGatewayFallback { .. } => {
                            tracing::warn!(
                                cid = %cid_owned,
                                error = %e,
                                "kubo cat failed; falling back to gateway"
                            );
                            return gateway_cat(&backend, &cid_owned).await;
                        }
                        _ => return Err(e),
                    },
                }
            }
            // Gateway-only mode.
            gateway_cat(&backend, &cid_owned).await
        })
    }

    /// Add bytes via kubo `/api/v0/add` and pin them. Returns the
    /// CID. Errors when the backend is in `Gateway`-only mode.
    pub fn add_and_pin(&self, body: Vec<u8>) -> Result<String> {
        let backend = self.clone();
        self.inner.runtime.block_on(async move {
            if backend.kubo_url().is_none() {
                return Err(RuLakeError::Backend {
                    backend: backend.id.clone(),
                    detail: "Mode::Gateway is read-only — publish_bundle requires Mode::Kubo or Mode::KuboWithGatewayFallback".into(),
                });
            }
            let cid = kubo_add(&backend, body).await?;
            kubo_pin_add(&backend, &cid).await?;
            Ok(cid)
        })
    }

    /// **Write side** — serialize the bundle to JSON, add+pin, record
    /// the resulting CID against the collection.
    pub fn publish_bundle(&self, collection: &str, bundle: &RuLakeBundle) -> Result<String> {
        let body = bundle.to_json()?;
        let cid = self.add_and_pin(body.into_bytes())?;
        self.record_cid(collection, cid.clone(), Some(bundle.dim));
        Ok(cid)
    }

    /// Read the on-IPFS bundle for a collection. Verifies the witness
    /// per `RuLakeBundle::from_json` (`src/bundle.rs:215+`) and
    /// surfaces a soft warning when the bundle's `data_ref` doesn't
    /// match the fetched CID.
    pub fn fetch_bundle(&self, collection: &str) -> Result<RuLakeBundle> {
        let c = self.collection(collection)?;
        let cid = c.cid.as_deref().ok_or_else(|| RuLakeError::Backend {
            backend: self.id.clone(),
            detail: format!("collection {collection:?} has no CID — call publish_bundle first or operator-supply a CID at register time"),
        })?;
        let body = self.cat(cid)?;
        let body_str = std::str::from_utf8(&body).map_err(|e| RuLakeError::Backend {
            backend: self.id.clone(),
            detail: format!("bundle bytes not utf-8: {e}"),
        })?;
        let bundle = RuLakeBundle::from_json(body_str)?;
        let expected = format!("ipfs://{cid}");
        if bundle.data_ref != expected {
            // Hard refuse — security finding R-IPFS-1. Witness equality
            // alone does not catch CID-substitution attacks where a
            // legitimately-witnessed bundle is re-pinned under a
            // different CID and served back to a caller that recorded
            // the original. The data_ref ↔ CID binding IS the trust
            // anchor; a mismatch means the operator is lying about
            // which bundle this is.
            tracing::error!(
                got = %bundle.data_ref,
                expected = %expected,
                "IPFS_BUNDLE_CID_MISMATCH: bundle data_ref does not match fetched CID — refusing"
            );
            return Err(RuLakeError::Backend {
                backend: self.id.clone(),
                detail: format!(
                    "IPFS_BUNDLE_CID_MISMATCH: bundle.data_ref={:?} but fetched cid={:?} — refusing to trust",
                    bundle.data_ref, expected
                ),
            });
        }
        Ok(bundle)
    }
}

// ─── BackendAdapter impl ─────────────────────────────────────────────

impl BackendAdapter for IpfsBackend {
    fn id(&self) -> &str {
        &self.id
    }

    fn list_collections(&self) -> Result<Vec<CollectionId>> {
        Ok(self
            .inner
            .collections
            .read()
            .unwrap()
            .keys()
            .cloned()
            .collect())
    }

    /// IPFS is the **bundle distribution layer**, not the vector-data
    /// layer (ADR-005 §1). `pull_vectors` errors — operators wire
    /// IPFS for the bundle round-trip and rely on the *original*
    /// backend (Parquet, GCS, etc.) for the vector bodies.
    fn pull_vectors(&self, collection: &str) -> Result<PulledBatch> {
        Err(RuLakeError::Backend {
            backend: self.id.clone(),
            detail: format!(
                "IpfsBackend is bundle-only (ADR-005 §1) — pull_vectors not supported for collection {collection:?}. \
                 Wire a body-store backend (gcs / fs / local) for the vectors and use this backend for the bundle witness."
            ),
        })
    }

    fn generation(&self, collection: &str) -> Result<u64> {
        let c = self.collection(collection)?;
        let cid = c.cid.as_deref().ok_or_else(|| RuLakeError::Backend {
            backend: self.id.clone(),
            detail: format!("collection {collection:?} has no CID"),
        })?;
        Ok(stable_hash_u64(cid.as_bytes()))
    }

    /// **The cheap-path override** (ADR-004 §Resources contract,
    /// ADR-005 §4). Fast path: operator pre-set CID + dim → no IPFS
    /// round trip; we synthesize the bundle locally. Otherwise one
    /// `cat` of the CID — bundles are single-block (≤ 64 KiB cap).
    fn current_bundle(
        &self,
        collection: &str,
        rotation_seed: u64,
        rerank_factor: usize,
    ) -> Result<RuLakeBundle> {
        let c = self.collection(collection)?;
        if let (Some(cid), Some(dim)) = (c.cid.as_deref(), c.dim) {
            let bundle = RuLakeBundle::new(
                format!("ipfs://{cid}"),
                dim,
                rotation_seed,
                rerank_factor,
                Generation::Opaque(cid.to_string()),
            );
            return Ok(bundle);
        }
        self.fetch_bundle(collection)
    }
}

// ─── HTTP helpers (kubo + gateway) ───────────────────────────────────

#[derive(Debug, Deserialize)]
struct KuboAddResponse {
    #[serde(rename = "Hash")]
    hash: String,
    #[serde(rename = "Name", default)]
    #[allow(dead_code)]
    name: String,
    #[serde(rename = "Size", default)]
    #[allow(dead_code)]
    size: String,
}

async fn kubo_cat(backend: &IpfsBackend, cid: &str) -> Result<Vec<u8>> {
    let url = format!(
        "{}/api/v0/cat?arg={}",
        backend.kubo_url().unwrap().trim_end_matches('/'),
        cid
    );
    let mut req = backend.inner.http.post(&url);
    if let Some(t) = backend.kubo_token() {
        req = req.bearer_auth(t);
    }
    let resp = req.send().await.map_err(|e| RuLakeError::Backend {
        backend: backend.id.clone(),
        detail: format!("kubo cat {cid}: {e}"),
    })?;
    if !resp.status().is_success() {
        return Err(RuLakeError::Backend {
            backend: backend.id.clone(),
            detail: format!("kubo cat {cid}: HTTP {}", resp.status()),
        });
    }
    let bytes = resp.bytes().await.map_err(|e| RuLakeError::Backend {
        backend: backend.id.clone(),
        detail: format!("kubo cat body: {e}"),
    })?;
    Ok(bytes.to_vec())
}

async fn kubo_add(backend: &IpfsBackend, body: Vec<u8>) -> Result<String> {
    let url = format!(
        "{}/api/v0/add?cid-version=1",
        backend.kubo_url().unwrap().trim_end_matches('/')
    );
    let part = reqwest::multipart::Part::bytes(body).file_name("bundle.json");
    let form = reqwest::multipart::Form::new().part("file", part);
    let mut req = backend.inner.http.post(&url).multipart(form);
    if let Some(t) = backend.kubo_token() {
        req = req.bearer_auth(t);
    }
    let resp = req.send().await.map_err(|e| RuLakeError::Backend {
        backend: backend.id.clone(),
        detail: format!("kubo add: {e}"),
    })?;
    if !resp.status().is_success() {
        return Err(RuLakeError::Backend {
            backend: backend.id.clone(),
            detail: format!("kubo add: HTTP {}", resp.status()),
        });
    }
    let text = resp.text().await.map_err(|e| RuLakeError::Backend {
        backend: backend.id.clone(),
        detail: format!("kubo add body: {e}"),
    })?;
    // kubo `add` may emit one JSON object per line; we want the last
    // object (which is the file we uploaded — earlier lines describe
    // intermediate dir entries when the input is a directory).
    let last = text
        .lines()
        .rfind(|l| !l.trim().is_empty())
        .ok_or_else(|| RuLakeError::Backend {
            backend: backend.id.clone(),
            detail: "kubo add: empty response".into(),
        })?;
    let parsed: KuboAddResponse = serde_json::from_str(last).map_err(|e| RuLakeError::Backend {
        backend: backend.id.clone(),
        detail: format!("kubo add: parse {last:?}: {e}"),
    })?;
    Ok(parsed.hash)
}

async fn kubo_pin_add(backend: &IpfsBackend, cid: &str) -> Result<()> {
    let url = format!(
        "{}/api/v0/pin/add?arg={}",
        backend.kubo_url().unwrap().trim_end_matches('/'),
        cid
    );
    let mut req = backend.inner.http.post(&url);
    if let Some(t) = backend.kubo_token() {
        req = req.bearer_auth(t);
    }
    let resp = req.send().await.map_err(|e| RuLakeError::Backend {
        backend: backend.id.clone(),
        detail: format!("kubo pin/add {cid}: {e}"),
    })?;
    if !resp.status().is_success() {
        return Err(RuLakeError::Backend {
            backend: backend.id.clone(),
            detail: format!("kubo pin/add {cid}: HTTP {}", resp.status()),
        });
    }
    Ok(())
}

async fn gateway_cat(backend: &IpfsBackend, cid: &str) -> Result<Vec<u8>> {
    let gw = backend.gateway_url().ok_or_else(|| RuLakeError::Backend {
        backend: backend.id.clone(),
        detail: "no gateway configured".into(),
    })?;
    let url = format!("{}/ipfs/{}", gw.trim_end_matches('/'), cid);
    let resp = backend
        .inner
        .http
        .get(&url)
        .send()
        .await
        .map_err(|e| RuLakeError::Backend {
            backend: backend.id.clone(),
            detail: format!("gateway GET {url}: {e}"),
        })?;
    if !resp.status().is_success() {
        return Err(RuLakeError::Backend {
            backend: backend.id.clone(),
            detail: format!("gateway GET {url}: HTTP {}", resp.status()),
        });
    }
    let bytes = resp.bytes().await.map_err(|e| RuLakeError::Backend {
        backend: backend.id.clone(),
        detail: format!("gateway body {url}: {e}"),
    })?;
    Ok(bytes.to_vec())
}

// ─── Helpers ─────────────────────────────────────────────────────────

fn stable_hash_u64(bytes: &[u8]) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    bytes.hash(&mut h);
    h.finish()
}

/// Surface-level CID sanity check. kubo / the gateway is the canonical
/// validator — we just catch the obvious cases (empty, whitespace,
/// control bytes) so a misconfig dies cleanly at register time.
pub fn validate_cid_shape(s: &str) -> Result<()> {
    if s.is_empty() {
        return Err(RuLakeError::InvalidParameter("CID is empty".into()));
    }
    for b in s.bytes() {
        if b <= 0x20 || b == 0x7f {
            return Err(RuLakeError::InvalidParameter(format!(
                "CID {s:?}: contains whitespace or control byte"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_cid_rejects_empty_and_whitespace() {
        assert!(validate_cid_shape("").is_err());
        assert!(validate_cid_shape("Qm 123").is_err());
        assert!(validate_cid_shape("Qm\x01x").is_err());
        assert!(validate_cid_shape("QmZ4tDuvesekSs4qM5ZBKpXiZGun7S2CYtEZRB3DYXkjGx").is_ok());
        assert!(
            validate_cid_shape("bafybeigdyrztktbnzdmnfvk7t6mjg6mbsj4mejxvgjlb6scxorsbcs6htu")
                .is_ok()
        );
    }
}
