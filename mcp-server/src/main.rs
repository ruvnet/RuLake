//! `rulake-mcp` binary — the MCP server entry point.
//!
//! v0.2 supports stdio + Streamable HTTP. OAuth, mTLS, replay
//! protection ship in v0.3 per ADR-004 §Open questions.

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::Context;
use jsonwebtoken::Algorithm;
use ruvector_rulake_mcp::{
    AllowBearerOnPublic, AuditSink, AuthMode, BearerAuth, CapabilitySet, InsecureAllowNoAuth,
    JwksKeys, JwtAuth, JwtConfig, McpConfig, RuLakeMcpServer,
};
use ruvector_rulake_mcp::auth::JwtKey;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let args = parse_args()?;

    let config = match args.config {
        Some(path) => McpConfig::from_path(&path)
            .with_context(|| format!("loading config: {}", path.display()))?,
        None => McpConfig::default(),
    };

    let capabilities = match args.capabilities.as_deref() {
        Some(csv) => CapabilitySet::from_csv(csv)?,
        None => CapabilitySet::default(),
    };
    tracing::info!(capabilities = ?capabilities.labels(), "starting rulake-mcp");
    let audit = match &args.audit_file {
        Some(p) => {
            let sink = AuditSink::open_file(p)
                .with_context(|| format!("opening audit file: {}", p.display()))?;
            tracing::info!(path = %p.display(), "audit → JSONL file");
            sink
        }
        None => {
            tracing::info!("audit → stderr (no --audit-file set)");
            AuditSink::stderr()
        }
    };
    let server = RuLakeMcpServer::new_with_caps(config, capabilities)?.with_audit(audit);

    match args.transport {
        Transport::Stdio => server.serve_stdio().await,
        Transport::Http(http) => {
            let auth = build_auth(&http).await?;
            ruvector_rulake_mcp::http::serve(
                server,
                http.bind,
                auth,
                AllowBearerOnPublic(http.allow_bearer_on_public),
                InsecureAllowNoAuth(http.insecure_allow_no_auth),
            )
            .await
        }
    }
}

async fn build_auth(http: &HttpArgs) -> anyhow::Result<AuthMode> {
    match http.auth.as_str() {
        "none" => Ok(AuthMode::None),
        "bearer" => {
            let path = http
                .bearer_token_file
                .as_ref()
                .context("--auth bearer requires --bearer-token-file PATH")?;
            let bearer = BearerAuth::from_file(path)
                .with_context(|| format!("loading bearer token: {}", path.display()))?;
            Ok(AuthMode::Bearer(bearer))
        }
        "jwt" => {
            let issuer = http
                .jwt_issuer
                .clone()
                .context("--auth jwt requires --jwt-issuer URL")?;
            let audience = http
                .jwt_audience
                .clone()
                .context("--auth jwt requires --jwt-audience URL (RFC 8707 Resource Indicator)")?;
            let alg = match http.jwt_alg.as_deref().unwrap_or("HS256") {
                "HS256" => Algorithm::HS256,
                "HS384" => Algorithm::HS384,
                "HS512" => Algorithm::HS512,
                "RS256" => Algorithm::RS256,
                "RS384" => Algorithm::RS384,
                "RS512" => Algorithm::RS512,
                "ES256" => Algorithm::ES256,
                "ES384" => Algorithm::ES384,
                other => anyhow::bail!(
                    "unknown --jwt-alg {other:?} — supported: HS256|HS384|HS512|RS256|RS384|RS512|ES256|ES384"
                ),
            };

            // Pick a key source — exactly one of: secret-file (HMAC),
            // rsa-pem-file (RS*), ec-pem-file (ES*), or jwks-url
            // (rotation set fetched in the background).
            let key = if let Some(p) = &http.jwt_secret_file {
                JwtKey::Hmac(
                    std::fs::read(p).with_context(|| format!("loading jwt secret: {}", p.display()))?,
                )
            } else if let Some(p) = &http.jwt_rsa_pem_file {
                JwtKey::RsaPem(
                    std::fs::read(p)
                        .with_context(|| format!("loading jwt rsa pem: {}", p.display()))?,
                )
            } else if let Some(p) = &http.jwt_ec_pem_file {
                JwtKey::EcPem(
                    std::fs::read(p)
                        .with_context(|| format!("loading jwt ec pem: {}", p.display()))?,
                )
            } else if let Some(url) = &http.jwt_jwks_url {
                let keys = JwksKeys::empty(url.clone());
                let http_client = reqwest::Client::new();
                let interval =
                    std::time::Duration::from_secs(http.jwt_jwks_refresh_secs.unwrap_or(300));
                ruvector_rulake_mcp::spawn_jwks_refresh(keys.clone(), interval, http_client)
                    .await
                    .with_context(|| format!("initial JWKS fetch from {url}"))?;
                tracing::info!(url = %url, "JWKS fetch loop active");
                JwtKey::Jwks(keys)
            } else {
                anyhow::bail!(
                    "--auth jwt requires exactly one of: --jwt-secret-file (HMAC), \
                     --jwt-rsa-pem-file (RS*), --jwt-ec-pem-file (ES*), or --jwt-jwks-url \
                     (rotation set fetched in the background)"
                );
            };

            let jwt = JwtAuth::new(JwtConfig {
                key,
                issuer,
                audience,
                algorithms: vec![alg],
            });
            Ok(AuthMode::Jwt(jwt))
        }
        other => anyhow::bail!(
            "unknown --auth mode {other:?} — expected `none` | `bearer` | `jwt` \
             (mtls + RS256 land in v0.5)"
        ),
    }
}

#[derive(Debug)]
struct Args {
    transport: Transport,
    config: Option<PathBuf>,
    capabilities: Option<String>,
    audit_file: Option<PathBuf>,
}

#[derive(Debug)]
enum Transport {
    Stdio,
    Http(HttpArgs),
}

#[derive(Debug)]
struct HttpArgs {
    bind: SocketAddr,
    auth: String,
    bearer_token_file: Option<PathBuf>,
    allow_bearer_on_public: bool,
    insecure_allow_no_auth: bool,
    jwt_secret_file: Option<PathBuf>,
    jwt_rsa_pem_file: Option<PathBuf>,
    jwt_ec_pem_file: Option<PathBuf>,
    jwt_jwks_url: Option<String>,
    jwt_jwks_refresh_secs: Option<u64>,
    jwt_issuer: Option<String>,
    jwt_audience: Option<String>,
    jwt_alg: Option<String>,
}

fn parse_args() -> anyhow::Result<Args> {
    let mut transport: Option<Transport> = None;
    let mut config = None;
    let mut capabilities: Option<String> = None;
    let mut audit_file: Option<PathBuf> = None;

    // For http subcommand:
    let mut http_bind: Option<SocketAddr> = None;
    let mut http_auth = "none".to_string();
    let mut http_token_file: Option<PathBuf> = None;
    let mut http_allow_bearer_on_public = false;
    let mut http_insecure_allow_no_auth = false;
    let mut http_jwt_secret_file: Option<PathBuf> = None;
    let mut http_jwt_rsa_pem_file: Option<PathBuf> = None;
    let mut http_jwt_ec_pem_file: Option<PathBuf> = None;
    let mut http_jwt_jwks_url: Option<String> = None;
    let mut http_jwt_jwks_refresh_secs: Option<u64> = None;
    let mut http_jwt_issuer: Option<String> = None;
    let mut http_jwt_audience: Option<String> = None;
    let mut http_jwt_alg: Option<String> = None;
    let mut transport_kind: Option<&'static str> = None;

    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "stdio" => transport_kind = Some("stdio"),
            "http" => transport_kind = Some("http"),
            "--config" => {
                config = Some(PathBuf::from(
                    it.next().context("--config expects a path")?,
                ));
            }
            "--capabilities" => {
                capabilities = Some(it.next().context("--capabilities expects CSV")?);
            }
            "--audit-file" => {
                audit_file = Some(PathBuf::from(
                    it.next().context("--audit-file expects PATH")?,
                ));
            }
            "--bind" => {
                let s = it.next().context("--bind expects ADDR:PORT")?;
                http_bind = Some(s.parse().with_context(|| format!("--bind {s:?}"))?);
            }
            "--auth" => {
                http_auth = it.next().context("--auth expects MODE (none|bearer)")?;
            }
            "--bearer-token-file" => {
                http_token_file = Some(PathBuf::from(
                    it.next().context("--bearer-token-file expects PATH")?,
                ));
            }
            "--allow-bearer-on-public" => http_allow_bearer_on_public = true,
            "--insecure-allow-no-auth" => http_insecure_allow_no_auth = true,
            "--jwt-secret-file" => {
                http_jwt_secret_file = Some(PathBuf::from(
                    it.next().context("--jwt-secret-file expects PATH")?,
                ));
            }
            "--jwt-issuer" => {
                http_jwt_issuer = Some(it.next().context("--jwt-issuer expects URL")?);
            }
            "--jwt-audience" => {
                http_jwt_audience = Some(it.next().context("--jwt-audience expects URL")?);
            }
            "--jwt-alg" => {
                http_jwt_alg = Some(
                    it.next()
                        .context("--jwt-alg expects HS256|HS384|HS512|RS256|RS384|RS512|ES256|ES384")?,
                );
            }
            "--jwt-rsa-pem-file" => {
                http_jwt_rsa_pem_file = Some(PathBuf::from(
                    it.next().context("--jwt-rsa-pem-file expects PATH")?,
                ));
            }
            "--jwt-ec-pem-file" => {
                http_jwt_ec_pem_file = Some(PathBuf::from(
                    it.next().context("--jwt-ec-pem-file expects PATH")?,
                ));
            }
            "--jwt-jwks-url" => {
                http_jwt_jwks_url = Some(it.next().context("--jwt-jwks-url expects URL")?);
            }
            "--jwt-jwks-refresh-secs" => {
                let s = it.next().context("--jwt-jwks-refresh-secs expects N")?;
                http_jwt_jwks_refresh_secs =
                    Some(s.parse().with_context(|| format!("--jwt-jwks-refresh-secs {s:?}"))?);
            }
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            other => anyhow::bail!("unknown arg: {other}"),
        }
    }

    transport = match transport_kind.unwrap_or("stdio") {
        "stdio" => Some(Transport::Stdio),
        "http" => {
            let bind = http_bind.unwrap_or_else(|| "127.0.0.1:7440".parse().unwrap());
            Some(Transport::Http(HttpArgs {
                bind,
                auth: http_auth,
                bearer_token_file: http_token_file,
                allow_bearer_on_public: http_allow_bearer_on_public,
                insecure_allow_no_auth: http_insecure_allow_no_auth,
                jwt_secret_file: http_jwt_secret_file,
                jwt_rsa_pem_file: http_jwt_rsa_pem_file,
                jwt_ec_pem_file: http_jwt_ec_pem_file,
                jwt_jwks_url: http_jwt_jwks_url,
                jwt_jwks_refresh_secs: http_jwt_jwks_refresh_secs,
                jwt_issuer: http_jwt_issuer,
                jwt_audience: http_jwt_audience,
                jwt_alg: http_jwt_alg,
            }))
        }
        _ => unreachable!(),
    };

    Ok(Args {
        transport: transport.unwrap(),
        config,
        capabilities,
        audit_file,
    })
}

fn print_help() {
    println!(
        "rulake-mcp — ruLake MCP server (ADR-004 v0.2)\n\
         \n\
         USAGE:\n    \
             rulake-mcp stdio [--config PATH]\n    \
             rulake-mcp http  [--config PATH] [--bind ADDR:PORT] [--auth MODE] [...]\n\
         \n\
         COMMON OPTIONS:\n    \
             --config PATH                  Load mcp.toml from PATH.\n    \
             --audit-file PATH              Append-only JSONL audit per ADR-004 §7. Default: stderr.\n    \
             --capabilities CSV             Tier set: read|internal|publish|admin (default: read).\n    \
                                            `read` exposes rulake_query + list_backends.\n    \
                                            `publish` adds publish_bundle + refresh_from_bundle_dir.\n    \
                                            `admin` adds save_cache_to_dir + warm_from_dir + invalidate_cache.\n    \
                                            `internal` exposes the kernel rulake_query composes (operator-only).\n    \
             -h, --help                     Print this help.\n\
         \n\
         HTTP OPTIONS:\n    \
             --bind ADDR:PORT               Bind address. Default 127.0.0.1:7440.\n    \
             --auth MODE                    `none` (loopback) | `bearer` (dev) | `jwt` (production).\n    \
             --bearer-token-file PATH       Required with --auth bearer.\n    \
             --allow-bearer-on-public       Required to bind --auth bearer to a non-loopback addr.\n    \
                                            BEARER IS DEV ONLY — static tokens leak once → permanent\n    \
                                            access. Use --auth jwt for production.\n    \
             --insecure-allow-no-auth       Required to bind --auth none to a non-loopback addr.\n    \
             --jwt-secret-file PATH         HMAC secret bytes (HS256/384/512).\n    \
             --jwt-rsa-pem-file PATH        RSA public key in PEM (RS256/384/512).\n    \
             --jwt-ec-pem-file PATH         EC public key in PEM (ES256/384).\n    \
             --jwt-jwks-url URL             Production: fetch JWKS from URL with hot key rotation.\n    \
             --jwt-jwks-refresh-secs N      JWKS refresh interval. Default 300s.\n    \
             --jwt-issuer URL               Required iss claim (--auth jwt).\n    \
             --jwt-audience URL             Required aud claim per RFC 8707 (--auth jwt).\n    \
             --jwt-alg ALG                  HS256 (default) | HS384 | HS512 | RS256 | RS384 | RS512 | ES256 | ES384.\n\
         \n\
         OAuth 2.1 + mTLS auth, replay protection (MCP-Request-Id +\n\
         session binding), and the full capability set (publish/admin)\n\
         ship in v0.3. See docs/adrs/sdk/ADR-004-rulake-mcp-server.md."
    );
}

fn init_tracing() {
    use tracing_subscriber::{EnvFilter, fmt, prelude::*};
    let _ = tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(fmt::layer().json().with_writer(std::io::stderr))
        .try_init();
}
