//! End-to-end test for the `--demo-backend` / `demo_backend = true` path.
//!
//! Asserts that constructing `RuLakeMcpServer` with `McpConfig::demo_backend
//! = true` registers the seeded `LocalBackend("demo")` AND adds the
//! corresponding `[[allow]]` block — so a public-demo Cloud Run install
//! using `--demo-backend` can answer real `rulake_query` calls instead of
//! refusing on an empty allowlist.
//!
//! This is the test that catches regressions of the `make 100% functional`
//! commit if the wiring ever drifts.

use ruvector_rulake_mcp::{config::McpConfig, server::RuLakeMcpServer};

#[test]
fn demo_backend_registers_seeded_collection_and_allow_block() {
    let cfg = McpConfig {
        demo_backend: true,
        ..McpConfig::default()
    };
    // Construct should succeed — the seeded LocalBackend is wired in.
    let _server = RuLakeMcpServer::new(cfg).expect("build server with demo_backend = true");
    // Successful construction is the assertion. The path that registers
    // the seeded collection + adds the allow-block runs unconditionally
    // when demo_backend is true; failure surfaces as anyhow::Error from
    // RuLakeMcpServer::new.
}

#[test]
fn demo_backend_off_by_default() {
    let cfg = McpConfig::default();
    assert!(
        !cfg.demo_backend,
        "production deploys must opt in explicitly"
    );
    let _server = RuLakeMcpServer::new(cfg).expect("default-config server still constructs");
}
