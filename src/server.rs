//! server — HTTP listener wiring the MCP Streamable HTTP transport.
//!
//! v0.1.3-alpha: the `/mcp` 501 stub is replaced by a real MCP server built on
//! rmcp's built-in **Streamable HTTP** server transport (`StreamableHttpService`
//! + `LocalSessionManager`), the transport GPT/Claude remote connectors speak.
//!
//! Routes:
//! - `GET /health`            → 200, no auth (tunnel-provider healthchecks)
//! - `ANY /mcp` (+ `/mcp/`)   → bearer auth required, then handed to rmcp
//!
//! Auth is a bearer-token check (constant-time, reusing [`crate::auth`]) applied
//! as an axum middleware layer wrapping only the `/mcp` routes. Successful and
//! failed authentications are appended to the audit logs under `.local-pass/`.
//!
//! Guardrails: the active tool profile (default `lean`) plus a configurable
//! file-root scope and optional `--read-only` mode — see [`crate::profile`] and
//! [`crate::guard`].

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::{
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Json, Response},
    routing::get,
    Router,
};
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use serde_json::json;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

use crate::auth;
use crate::guard::Guard;
use crate::mcp::{self, GatewayHandler};
use crate::profile::Profile;

/// Parsed `serve` options.
struct ServeOpts {
    bind: SocketAddr,
    profile: String,
    root: Option<String>,
    read_only: bool,
}

/// State shared with the bearer-auth middleware.
#[derive(Clone)]
struct AuthState {
    bearer_token: Arc<String>,
    access_log: Arc<Mutex<PathBuf>>,
    auth_fail_log: Arc<Mutex<PathBuf>>,
}

pub fn run(args: &[String]) -> Result<()> {
    let opts = parse_opts(args)?;

    // Validate the profile up front so a typo fails loudly instead of exposing
    // an unexpected surface.
    let profile =
        Profile::resolve(&opts.profile).map_err(|e| anyhow::anyhow!("invalid --profile: {e}"))?;

    let token = auth::read_token()
        .context("could not load bearer token")?
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no auth token found at {}\n\nRun `local-pass init` first to generate a bearer token.",
                auth::token_path()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default()
            )
        })?;

    let state_dir = auth::token_path()?
        .parent()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| anyhow::anyhow!("could not resolve .local-pass state dir"))?;

    let root = mcp::resolve_root(opts.root.clone());
    let guard = Guard::new(root, opts.read_only);

    let auth_state = AuthState {
        bearer_token: Arc::new(token),
        access_log: Arc::new(Mutex::new(state_dir.join("access.log"))),
        auth_fail_log: Arc::new(Mutex::new(state_dir.join("auth_failures.log"))),
    };

    // Build the runtime explicitly (main.rs is sync).
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to build tokio runtime")?;

    rt.block_on(async move {
        // rmcp Streamable HTTP service: a fresh GatewayHandler per session,
        // backed by the bundled in-memory session manager. stateful_mode=true
        // enables the Mcp-Session-Id lifecycle (GET resume + DELETE teardown)
        // that remote connectors rely on.
        let handler_profile = profile.clone();
        let handler_guard = guard.clone();
        let mcp_service = StreamableHttpService::new(
            move || {
                Ok(GatewayHandler::new(
                    handler_profile.clone(),
                    handler_guard.clone(),
                ))
            },
            Arc::new(LocalSessionManager::default()),
            StreamableHttpServerConfig::default(),
        );

        // A single nest at "/mcp" serves both the exact path and "/mcp/..." —
        // nesting "/mcp" and "/mcp/" separately collides on axum's internal
        // tail-param route.
        let mcp_routes =
            Router::new()
                .nest_service("/mcp", mcp_service)
                .layer(middleware::from_fn_with_state(
                    auth_state.clone(),
                    bearer_auth,
                ));

        let app = Router::new()
            .route("/health", get(health_handler))
            .merge(mcp_routes)
            .fallback(not_found_handler);

        let listener = tokio::net::TcpListener::bind(&opts.bind)
            .await
            .with_context(|| format!("failed to bind {}", opts.bind))?;

        eprintln!(
            "Local-Pass v{} listening on http://{}",
            env!("CARGO_PKG_VERSION"),
            opts.bind
        );
        eprintln!(
            "Bearer token loaded from {}",
            auth::token_path().unwrap_or_default().display()
        );
        eprintln!(
            "Profile:    {} ({} tools; tools/list + tools/call filtered)",
            profile.name(),
            profile.allowed().len()
        );
        eprintln!("Safe root:  {}", guard.root().display());
        eprintln!(
            "Mode:       {}",
            if guard.is_read_only() {
                "read-only (write_file + shell denied)"
            } else {
                "read-write"
            }
        );
        eprintln!("Endpoints:");
        eprintln!("  GET  /health   (no auth) — healthcheck for tunnel providers");
        eprintln!("  ANY  /mcp      (bearer auth) — MCP Streamable HTTP endpoint");
        eprintln!("Press Ctrl+C to stop.");

        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("HTTP server error")
    })?;

    eprintln!("Local-Pass server stopped.");
    Ok(())
}

fn parse_opts(args: &[String]) -> Result<ServeOpts> {
    let mut bind = "127.0.0.1:9100".to_string();
    let mut profile = "lean".to_string();
    let mut root: Option<String> = None;
    let mut read_only = false;

    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "--bind" | "-b" => {
                bind = next_value(args, &mut i, "--bind")?;
            }
            _ if a.starts_with("--bind=") => bind = a["--bind=".len()..].to_string(),
            "--profile" | "-p" => {
                profile = next_value(args, &mut i, "--profile")?;
            }
            _ if a.starts_with("--profile=") => profile = a["--profile=".len()..].to_string(),
            "--root" | "-r" => {
                root = Some(next_value(args, &mut i, "--root")?);
            }
            _ if a.starts_with("--root=") => root = Some(a["--root=".len()..].to_string()),
            "--read-only" => read_only = true,
            _ => {}
        }
        i += 1;
    }

    let bind_addr: SocketAddr = bind.parse().with_context(|| {
        format!("invalid --bind value '{bind}' (expected ip:port like 127.0.0.1:9100)")
    })?;

    Ok(ServeOpts {
        bind: bind_addr,
        profile,
        root,
        read_only,
    })
}

/// Pull the value following a flag, advancing the index past it.
fn next_value(args: &[String], i: &mut usize, flag: &str) -> Result<String> {
    let v = args
        .get(*i + 1)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("{flag} requires a value"))?;
    *i += 1;
    Ok(v)
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c().await.ok();
    eprintln!("\nShutdown signal received; closing connections...");
}

// --- bearer auth middleware (only wraps /mcp) -------------------------------

async fn bearer_auth(
    State(state): State<AuthState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    request: Request,
    next: Next,
) -> Response {
    if let Err(reason) = check_bearer(request.headers(), &state.bearer_token) {
        log_auth_failure(&state, peer, reason).await;
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({
                "error": "unauthorized",
                "hint": "send Authorization: Bearer <token> header (token from `local-pass init`)"
            })),
        )
            .into_response();
    }
    log_access(&state, peer).await;
    next.run(request).await
}

fn check_bearer(headers: &HeaderMap, expected: &str) -> Result<(), &'static str> {
    let auth_header = headers
        .get("authorization")
        .ok_or("missing Authorization header")?;
    let auth_str = auth_header
        .to_str()
        .map_err(|_| "non-ASCII Authorization header")?;
    let token = auth_str
        .strip_prefix("Bearer ")
        .ok_or("Authorization header must start with 'Bearer '")?;
    if constant_time_eq(token.as_bytes(), expected.as_bytes()) {
        Ok(())
    } else {
        Err("bearer token mismatch")
    }
}

/// Constant-time comparison to prevent timing attacks on token validation.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut acc: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        acc |= x ^ y;
    }
    acc == 0
}

// --- handlers ---------------------------------------------------------------

async fn health_handler() -> impl IntoResponse {
    Json(json!({
        "status": "ok",
        "service": "local-pass",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

async fn not_found_handler() -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        Json(json!({
            "error": "not found",
            "hint": "valid endpoints: GET /health, POST /mcp"
        })),
    )
}

// --- audit log helpers ------------------------------------------------------

async fn log_access(state: &AuthState, peer: SocketAddr) {
    let line = format!(
        "{} access /mcp peer={}\n",
        chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ"),
        peer
    );
    let path = state.access_log.lock().await.clone();
    let _ = append_line(&path, &line).await;
}

async fn log_auth_failure(state: &AuthState, peer: SocketAddr, reason: &str) {
    let line = format!(
        "{} auth_fail peer={} reason={}\n",
        chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ"),
        peer,
        reason
    );
    let path = state.auth_fail_log.lock().await.clone();
    let _ = append_line(&path, &line).await;
}

async fn append_line(path: &PathBuf, line: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.ok();
    }
    let mut f = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await
        .with_context(|| format!("could not open log file: {}", path.display()))?;
    f.write_all(line.as_bytes())
        .await
        .with_context(|| format!("could not write to log file: {}", path.display()))?;
    Ok(())
}
