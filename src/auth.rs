//! auth — bearer-token generation, storage, and rotation for Local-Pass.
//!
//! Token format: 32 cryptographically random bytes, hex-encoded (64 ASCII chars).
//! Storage path: `<exe-dir>/.local-pass/auth.token` (co-located with the binary
//! so the install is fully portable — copy the folder, the token goes with it).
//!
//! Subcommands:
//!   init             generate token, write to disk, print to console (errors if exists unless --force)
//!   rotate-token     generate new token, overwrite existing, print to console (no backup — rotation = invalidation)

use anyhow::{Context, Result, bail};
use rand::RngCore;
use std::path::PathBuf;

const TOKEN_BYTES: usize = 32;

pub fn init(args: &[String]) -> Result<()> {
    let force = args.iter().any(|a| a == "--force" || a == "-f");

    let path = token_path()?;
    if path.exists() && !force {
        bail!(
            "token file already exists at {}\n\nRefusing to overwrite without --force (would invalidate your existing remote-AI sessions).\nIf you really want to regenerate, run: local-pass init --force\nIf you want to rotate (intentional invalidation), run: local-pass rotate-token",
            path.display()
        );
    }

    let token = generate_token();
    write_token_atomic(&path, &token)?;

    println!("Initialized bearer token for Local-Pass.");
    println!();
    println!("Token saved to: {}", path.display());
    println!("Token (use as Authorization: Bearer <token> in your AI client):");
    println!();
    println!("    {}", token);
    println!();
    println!("Add this header in Claude.ai integrations or ChatGPT custom actions when configuring the MCP server URL.");
    println!("Treat this token like an SSH private key — anyone with it can act on your machine.");
    Ok(())
}

pub fn rotate(_args: &[String]) -> Result<()> {
    let path = token_path()?;
    let existed = path.exists();

    let token = generate_token();
    write_token_atomic(&path, &token)?;

    println!("Rotated bearer token for Local-Pass.");
    println!();
    if existed {
        println!("Previous token has been INVALIDATED. Any AI client still using it will get 401 on the next request.");
    } else {
        println!("(No previous token existed — this is effectively the same as `local-pass init`.)");
    }
    println!();
    println!("New token saved to: {}", path.display());
    println!("New token:");
    println!();
    println!("    {}", token);
    println!();
    println!("Update your AI clients with the new bearer token, then they'll work again.");
    Ok(())
}

fn generate_token() -> String {
    let mut bytes = [0u8; TOKEN_BYTES];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex_encode(&bytes)
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

/// Resolve the token storage path: `<exe-dir>/.local-pass/auth.token`
pub fn token_path() -> Result<PathBuf> {
    let exe = std::env::current_exe()
        .context("could not resolve current executable path")?;
    let exe_dir = exe.parent()
        .with_context(|| format!("exe path has no parent: {}", exe.display()))?;
    Ok(exe_dir.join(".local-pass").join("auth.token"))
}

/// Read the current token, if it exists.
#[allow(dead_code)]
pub fn read_token() -> Result<Option<String>> {
    let path = token_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("read failed: {}", path.display()))?;
    Ok(Some(text.trim().to_string()))
}

/// Atomic write: write to .tmp sibling first, then rename to final path.
/// Avoids leaving a half-written token file if the process is killed mid-write.
fn write_token_atomic(path: &PathBuf, token: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("could not create parent dir: {}", parent.display()))?;
    }
    let tmp = path.with_extension("token.tmp");
    std::fs::write(&tmp, token)
        .with_context(|| format!("write failed: {}", tmp.display()))?;
    // On Windows, rename over an existing file requires the target to be removable;
    // std::fs::rename doesn't auto-replace pre-Rust-1.79 — use copy+remove fallback.
    if path.exists() {
        std::fs::remove_file(path)
            .with_context(|| format!("could not remove existing token at {}", path.display()))?;
    }
    std::fs::rename(&tmp, path)
        .with_context(|| format!("rename failed: {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}
