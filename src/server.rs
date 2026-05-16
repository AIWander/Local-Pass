//! server — HTTP listener + bearer auth + audit log + MCP endpoint stub.
//!
//! v0.1.2-alpha: minimum-viable HTTP transport.
//! - GET /health  → 200 (no auth, for tunnel-provider healthchecks)
//! - POST /mcp    → bearer auth required; returns 501 Not Implemented (full MCP-over-HTTP shape coming next)
//! - Every authenticated request appended to <exe-dir>/.local-pass/access.log
//! - Every failed-auth attempt appended to <exe-dir>/.local-pass/auth_failures.log
//! - Graceful shutdown on Ctrl+C.

use anyhow::{Context, Result};
use axum::{
    body::Bytes,
    extract::{ConnectInfo, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json},
    routing::{get, post},
    Router,
};
use serde_json::json;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::io::AsyncWriteExt;

use crate::auth;

#[derive(Clone)]
struct AppState {
    bearer_token: Arc<String>,
    access_log: Arc<Mutex<PathBuf>>,
    auth_fail_log: Arc<Mutex<PathBuf>>,
}

pub fn run(args: &[String]) -> Result<()> {
    let bind = parse_bind_arg(args).unwrap_or_else(|| "127.0.0.1:9100".to_string());
    let bind_addr: SocketAddr = bind.parse()
        .with_context(|| format!("invalid --bind value '{}' (expected ip:port like 127.0.0.1:9100)", bind))?;

    let token = auth::read_token()
        .context("could not load bearer token")?
        .ok_or_else(|| anyhow::anyhow!(
            "no auth token found at {}\n\nRun `local-pass init` first to generate a bearer token.",
            auth::token_path().map(|p| p.display().to_string()).unwrap_or_default()
        ))?;

    let state_dir = auth::token_path()?
        .parent()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| anyhow::anyhow!("could not resolve .local-pass state dir"))?;

    let state = AppState {
        bearer_token: Arc::new(token),
        access_log: Arc::new(Mutex::new(state_dir.join("access.log"))),
        auth_fail_log: Arc::new(Mutex::new(state_dir.join("auth_failures.log"))),
    };

    // Build the runtime explicitly (we don't use #[tokio::main] because main.rs is sync).
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to build tokio runtime")?;

    rt.block_on(async move {
        let app = Router::new()
            .route("/health", get(health_handler))
            .route("/mcp", post(mcp_handler))
            .fallback(not_found_handler)
            .with_state(state);

        let listener = tokio::net::TcpListener::bind(&bind_addr).await
            .with_context(|| format!("failed to bind {}", bind_addr))?;

        eprintln!("Local-Pass v{} listening on http://{}", env!("CARGO_PKG_VERSION"), bind_addr);
        eprintln!("Bearer token loaded from {}", auth::token_path().unwrap_or_default().display());
        eprintln!("Endpoints:");
        eprintln!("  GET  /health   (no auth) — healthcheck for tunnel providers");
        eprintln!("  POST /mcp      (bearer auth required) — MCP-over-HTTP endpoint (501 stub in v0.1.2-alpha)");
        eprintln!("Press Ctrl+C to stop.");

        axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
            .with_graceful_shutdown(shutdown_signal())
            .await
            .context("HTTP server error")
    })?;

    eprintln!("Local-Pass server stopped.");
    Ok(())
}

fn parse_bind_arg(args: &[String]) -> Option<String> {
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--bind" | "-b" => return args.get(i + 1).cloned(),
            s if s.starts_with("--bind=") => return Some(s[7..].to_string()),
            _ => i += 1,
        }
    }
    None
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.ok();
    };
    ctrl_c.await;
    eprintln!("\nShutdown signal received; closing connections...");
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
    (StatusCode::NOT_FOUND, Json(json!({
        "error": "not found",
        "hint": "valid endpoints: GET /health, POST /mcp"
    })))
}

async fn mcp_handler(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    if let Err(reason) = check_bearer(&headers, &state.bearer_token) {
        log_auth_failure(&state, peer, reason).await;
        return (StatusCode::UNAUTHORIZED, Json(json!({
            "error": "unauthorized",
            "hint": "send Authorization: Bearer <token> header (token from `local-pass init`)"
        })));
    }
    log_access(&state, peer, body.len()).await;
    (StatusCode::NOT_IMPLEMENTED, Json(json!({
        "error": "not implemented",
        "note": "Local-Pass v0.1.2-alpha: bearer auth + HTTP transport are live, but the full MCP-over-HTTP protocol handler ships in v0.1.3+. Tools can't be invoked yet.",
        "auth": "ok"
    })))
}

fn check_bearer(headers: &HeaderMap, expected: &str) -> Result<(), &'static str> {
    let auth_header = headers.get("authorization").ok_or("missing Authorization header")?;
    let auth_str = auth_header.to_str().map_err(|_| "non-ASCII Authorization header")?;
    let token = auth_str.strip_prefix("Bearer ").ok_or("Authorization header must start with 'Bearer '")?;
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

// --- audit log helpers ------------------------------------------------------

async fn log_access(state: &AppState, peer: SocketAddr, body_bytes: usize) {
    let line = format!(
        "{} access POST /mcp peer={} body_bytes={}\n",
        chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ"),
        peer,
        body_bytes
    );
    let path = state.access_log.lock().await.clone();
    let _ = append_line(&path, &line).await;
}

async fn log_auth_failure(state: &AppState, peer: SocketAddr, reason: &str) {
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
    f.write_all(line.as_bytes()).await
        .with_context(|| format!("could not write to log file: {}", path.display()))?;
    Ok(())
}
