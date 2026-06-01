//! mcp_tools — the lean set of tools the gateway embeds and executes.
//!
//! Adapted from the donor `local` MCP server (`C:\github\local\src\tools`),
//! trimmed to the curated remote surface and rewired through Local-Pass's
//! [`crate::guard::Guard`] so every filesystem path is root-scoped and shell /
//! write tools honor `--read-only`. Internal donor deps (auto_backup, the
//! Volumes-aware `smart_exec` fallback table, the donor `security` denylist)
//! are dropped or inlined — Local-Pass's containment guard is the boundary.
//!
//! Each tool is defined as a JSON-Schema `inputSchema` (consumed by
//! `tools/list`) plus a synchronous executor returning a `serde_json::Value`.
//! The executor is intentionally blocking and is driven from the async MCP
//! handler via `spawn_blocking`.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::process::Command;

use crate::guard::Guard;
use crate::psession;

#[cfg(windows)]
use std::os::windows::process::CommandExt;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// One embedded tool: its MCP schema plus a marker for whether it mutates state
/// (used by `--read-only` enforcement) and whether it touches the filesystem.
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub schema: Value,
}

/// All tools the gateway knows how to execute. The active [`crate::profile::Profile`]
/// filters this list for `tools/list` and `tools/call`.
pub fn all_specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "read_file",
            description: "Read a UTF-8 text file inside the safe root. Optional: search='pattern' returns only matching lines; lines='start:end' returns a 1-indexed range; max_kb caps full reads (default 200).",
            schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path (absolute under the safe root, or relative to it)" },
                    "search": { "type": "string", "description": "Optional: return only lines containing this substring (case-insensitive)" },
                    "lines": { "type": "string", "description": "Optional: line range like '50:100' (1-indexed)" },
                    "max_kb": { "type": "integer", "description": "Optional: max KB to return for a full read (default 200)" }
                },
                "required": ["path"]
            }),
        },
        ToolSpec {
            name: "write_file",
            description: "Write a UTF-8 text file inside the safe root (creates parent dirs). Denied in --read-only mode.",
            schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path (absolute under the safe root, or relative to it)" },
                    "content": { "type": "string", "description": "Full file content to write" }
                },
                "required": ["path", "content"]
            }),
        },
        ToolSpec {
            name: "list_dir",
            description: "List a directory tree inside the safe root, up to `depth` levels (default 2).",
            schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Directory path (absolute under the safe root, or relative to it)" },
                    "depth": { "type": "integer", "description": "Levels to descend (default 2)" }
                },
                "required": ["path"]
            }),
        },
        ToolSpec {
            name: "search_file",
            description: "Search for files by name or by content under a directory inside the safe root.",
            schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Directory to search (absolute under the safe root, or relative to it)" },
                    "pattern": { "type": "string", "description": "Substring/glob fragment to match" },
                    "search_type": { "type": "string", "enum": ["files", "content"], "default": "files", "description": "Match file names ('files') or file contents ('content')" },
                    "max_results": { "type": "integer", "description": "Cap on results (default 100)" }
                },
                "required": ["path", "pattern"]
            }),
        },
        ToolSpec {
            name: "smart_exec",
            description: "Run a single shell command once and return stdout/stderr/exit code. Uses PowerShell when PS syntax is detected, otherwise cmd. One-shot (no persistent state). Denied in --read-only mode.",
            schema: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "Command line to execute" },
                    "cwd": { "type": "string", "description": "Optional working directory (must be inside the safe root)" },
                    "timeout_secs": { "type": "integer", "description": "Timeout in seconds (default 60)" }
                },
                "required": ["command"]
            }),
        },
        ToolSpec {
            name: "http_request",
            description: "Make an outbound HTTP request (GET/POST/PUT/DELETE/PATCH/HEAD). Returns status, headers, body (capped), and timing.",
            schema: json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "Absolute URL" },
                    "method": { "type": "string", "default": "GET", "description": "HTTP method" },
                    "headers": { "type": "object", "description": "Request headers as key/value pairs" },
                    "body": { "type": "string", "description": "Request body (POST/PUT/PATCH)" },
                    "timeout_secs": { "type": "integer", "default": 30, "description": "Timeout in seconds" }
                },
                "required": ["url"]
            }),
        },
        ToolSpec {
            name: "psession_create",
            description: "Create a persistent PowerShell session that survives across calls (state: cwd, env, variables). Returns a session_id. Denied in --read-only mode.",
            schema: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "default": "default", "description": "Session label" },
                    "cwd": { "type": "string", "description": "Optional working directory (must be inside the safe root)" }
                }
            }),
        },
        ToolSpec {
            name: "psession_run",
            description: "Run a command in a persistent PowerShell session. State persists between calls. Denied in --read-only mode.",
            schema: json!({
                "type": "object",
                "properties": {
                    "session_id": { "type": "string", "description": "Session id from psession_create" },
                    "command": { "type": "string", "description": "Command to run" },
                    "timeout_secs": { "type": "integer", "default": 30, "description": "Timeout in seconds" }
                },
                "required": ["session_id", "command"]
            }),
        },
        ToolSpec {
            name: "psession_destroy",
            description: "Terminate a persistent PowerShell session.",
            schema: json!({
                "type": "object",
                "properties": {
                    "session_id": { "type": "string", "description": "Session id to destroy" }
                },
                "required": ["session_id"]
            }),
        },
        ToolSpec {
            name: "psession_list",
            description: "List active persistent PowerShell sessions.",
            schema: json!({ "type": "object", "properties": {} }),
        },
        ToolSpec {
            name: "psession_read",
            description: "Read the tail of a persistent session's output buffer.",
            schema: json!({
                "type": "object",
                "properties": {
                    "session_id": { "type": "string", "description": "Session id" },
                    "tail": { "type": "integer", "default": 20, "description": "Lines from the end" }
                },
                "required": ["session_id"]
            }),
        },
    ]
}

/// Execute a tool by name. The caller has already confirmed `name` is permitted
/// by the active profile; here we additionally enforce the [`Guard`] (root
/// scope + read-only). Returns `(value, is_error)`.
pub fn execute(name: &str, args: &Value, guard: &Guard) -> (Value, bool) {
    match name {
        "read_file" => read_file(args, guard),
        "write_file" => write_file(args, guard),
        "list_dir" => list_dir(args, guard),
        "search_file" => search_file(args, guard),
        "smart_exec" => smart_exec(args, guard),
        "http_request" => http_request(args),
        "psession_create" => psession_create(args, guard),
        "psession_run" => psession_run(args, guard),
        "psession_destroy" => (psession::execute("psession_destroy", args), false),
        "psession_list" => (psession::execute("psession_list", args), false),
        "psession_read" => (psession::execute("psession_read", args), false),
        other => (
            json!({ "error": format!("tool '{other}' is not embedded in this gateway") }),
            true,
        ),
    }
}

// --- filesystem tools -------------------------------------------------------

fn read_file(args: &Value, guard: &Guard) -> (Value, bool) {
    let raw = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
    let path = match guard.resolve_in_root(raw) {
        Ok(p) => p,
        Err(e) => return (json!({ "error": e }), true),
    };
    let search = args.get("search").and_then(|v| v.as_str());
    let lines = args.get("lines").and_then(|v| v.as_str());
    let max_kb = args.get("max_kb").and_then(|v| v.as_i64()).unwrap_or(200);

    if !path.exists() {
        return (json!({ "error": format!("file not found: {raw}") }), true);
    }

    if let Some(pattern) = search {
        let file = match fs::File::open(&path) {
            Ok(f) => f,
            Err(e) => return (json!({ "error": e.to_string() }), true),
        };
        let needle = pattern.to_lowercase();
        let mut matches: Vec<String> = Vec::new();
        let mut total = 0usize;
        for (i, line) in BufReader::new(file).lines().enumerate() {
            total = i + 1;
            if let Ok(text) = line {
                if text.to_lowercase().contains(&needle) {
                    matches.push(format!("{}:{}", i + 1, text));
                    if matches.len() >= 200 {
                        matches.push("[...truncated at 200 matches]".to_string());
                        break;
                    }
                }
            }
        }
        return (
            json!({ "matches": matches.len(), "lines_scanned": total, "content": matches.join("\n") }),
            false,
        );
    }

    if let Some(range) = lines {
        let parts: Vec<&str> = range.split(':').collect();
        if parts.len() != 2 {
            return (json!({ "error": "lines format: 'start:end'" }), true);
        }
        let start: usize = parts[0].parse().unwrap_or(1);
        let end: usize = parts[1].parse().unwrap_or(start);
        if start < 1 || end < start {
            return (json!({ "error": "invalid line range" }), true);
        }
        let file = match fs::File::open(&path) {
            Ok(f) => f,
            Err(e) => return (json!({ "error": e.to_string() }), true),
        };
        let mut out = Vec::new();
        for (i, line) in BufReader::new(file).lines().enumerate() {
            let n = i + 1;
            if n >= start && n <= end {
                if let Ok(text) = line {
                    out.push(format!("{n}:{text}"));
                }
            }
            if n > end {
                break;
            }
        }
        return (json!({ "content": out.join("\n") }), false);
    }

    match fs::read_to_string(&path) {
        Ok(content) => {
            let kb = content.len() as i64 / 1024;
            if kb > max_kb {
                let limit = (max_kb * 1024) as usize;
                let truncated: String = content.chars().take(limit).collect();
                (
                    json!({
                        "content": truncated,
                        "truncated": true,
                        "file_kb": kb,
                        "hint": "file larger than max_kb; use search= or lines= for targeted reads"
                    }),
                    false,
                )
            } else {
                (json!({ "content": content }), false)
            }
        }
        Err(e) => (json!({ "error": e.to_string() }), true),
    }
}

fn write_file(args: &Value, guard: &Guard) -> (Value, bool) {
    if let Err(e) = guard.deny_if_read_only("write_file") {
        return (json!({ "error": e }), true);
    }
    let raw = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
    let path = match guard.resolve_in_root(raw) {
        Ok(p) => p,
        Err(e) => return (json!({ "error": e }), true),
    };
    let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");

    if let Some(parent) = path.parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            return (json!({ "error": e.to_string() }), true);
        }
    }
    match fs::write(&path, content) {
        Ok(()) => (
            json!({ "written": path.display().to_string(), "bytes": content.len() }),
            false,
        ),
        Err(e) => (json!({ "error": e.to_string() }), true),
    }
}

fn list_dir(args: &Value, guard: &Guard) -> (Value, bool) {
    let raw = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
    let path = match guard.resolve_in_root(raw) {
        Ok(p) => p,
        Err(e) => return (json!({ "error": e }), true),
    };
    let depth = args.get("depth").and_then(|v| v.as_i64()).unwrap_or(2) as usize;
    let mut out = Vec::new();
    list_recursive(&path, depth, 0, &mut out);
    (
        json!({ "tree": out.join("\n"), "entries": out.len() }),
        false,
    )
}

fn list_recursive(base: &std::path::Path, max_depth: usize, depth: usize, out: &mut Vec<String>) {
    if depth > max_depth {
        return;
    }
    let entries = match fs::read_dir(base) {
        Ok(e) => e,
        Err(_) => return,
    };
    let mut items: Vec<_> = entries.filter_map(|e| e.ok()).collect();
    items.sort_by_key(|e| e.file_name());
    for entry in items.iter().take(500) {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let prefix = "  ".repeat(depth);
        if path.is_dir() {
            out.push(format!("{prefix}{name}/"));
            if depth < max_depth {
                list_recursive(&path, max_depth, depth + 1, out);
            }
        } else {
            out.push(format!("{prefix}{name}"));
        }
    }
}

fn search_file(args: &Value, guard: &Guard) -> (Value, bool) {
    let raw = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
    let root = match guard.resolve_in_root(raw) {
        Ok(p) => p,
        Err(e) => return (json!({ "error": e }), true),
    };
    let pattern = args.get("pattern").and_then(|v| v.as_str()).unwrap_or("");
    if pattern.is_empty() {
        return (json!({ "error": "pattern is required" }), true);
    }
    let search_type = args
        .get("search_type")
        .and_then(|v| v.as_str())
        .unwrap_or("files");
    let max_results = args
        .get("max_results")
        .and_then(|v| v.as_u64())
        .unwrap_or(100) as usize;

    let needle = pattern.to_lowercase();
    let mut results: Vec<Value> = Vec::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        if results.len() >= max_results {
            break;
        }
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.filter_map(|e| e.ok()) {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            if search_type == "files" {
                let name = entry.file_name().to_string_lossy().to_lowercase();
                if name.contains(&needle) {
                    results.push(json!({ "path": p.display().to_string() }));
                }
            } else {
                // content search: scan text files only, skip binary-ish reads
                if let Ok(content) = fs::read_to_string(&p) {
                    for (i, line) in content.lines().enumerate() {
                        if line.to_lowercase().contains(&needle) {
                            results.push(json!({
                                "path": p.display().to_string(),
                                "line": i + 1,
                                "text": line.trim()
                            }));
                            break;
                        }
                    }
                }
            }
            if results.len() >= max_results {
                break;
            }
        }
    }
    let count = results.len();
    (json!({ "results": results, "count": count }), false)
}

// --- shell tool -------------------------------------------------------------

fn smart_exec(args: &Value, guard: &Guard) -> (Value, bool) {
    if let Err(e) = guard.deny_if_read_only("shell execution") {
        return (json!({ "error": e }), true);
    }
    let command = args.get("command").and_then(|v| v.as_str()).unwrap_or("");
    if command.is_empty() {
        return (json!({ "error": "command is required" }), true);
    }
    // Optional cwd is root-scoped like any other path.
    let cwd = match args.get("cwd").and_then(|v| v.as_str()) {
        Some(c) => match guard.resolve_in_root(c) {
            Ok(p) => Some(p),
            Err(e) => return (json!({ "error": e }), true),
        },
        None => None,
    };

    let needs_powershell = command.contains('$')
        || command.contains("Get-")
        || command.contains("Set-")
        || command.contains("New-Item")
        || command.contains("Remove-Item")
        || command.contains("Where-Object")
        || command.contains("Select-Object")
        || command.contains("-ErrorAction")
        || command.contains("Format-Table");

    let mut cmd = if needs_powershell {
        let mut c = Command::new("powershell");
        c.args(["-NoProfile", "-NonInteractive", "-Command", command]);
        c
    } else {
        let mut c = Command::new("cmd");
        c.args(["/C", command]);
        c
    };
    if let Some(dir) = &cwd {
        cmd.current_dir(dir);
    }
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    match cmd.output() {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let success = output.status.success();
            (
                json!({
                    "shell": if needs_powershell { "powershell" } else { "cmd" },
                    "exit_code": output.status.code().unwrap_or(-1),
                    "stdout": stdout,
                    "stderr": stderr,
                    "success": success
                }),
                !success,
            )
        }
        Err(e) => (json!({ "error": e.to_string() }), true),
    }
}

// --- persistent sessions (root-scoped cwd, read-only gating) ----------------

fn psession_create(args: &Value, guard: &Guard) -> (Value, bool) {
    if let Err(e) = guard.deny_if_read_only("psession_create") {
        return (json!({ "error": e }), true);
    }
    // Re-scope cwd into the safe root if provided.
    let mut scoped = args.clone();
    if let Some(c) = args.get("cwd").and_then(|v| v.as_str()) {
        match guard.resolve_in_root(c) {
            Ok(p) => {
                scoped["cwd"] = json!(p.display().to_string());
            }
            Err(e) => return (json!({ "error": e }), true),
        }
    } else {
        // Default the session cwd to the safe root rather than the process CWD.
        scoped["cwd"] = json!(guard.root().display().to_string());
    }
    // Force the PowerShell backend; the donor's WSL backend is not exposed.
    scoped["shell"] = json!("powershell");
    (psession::execute("psession_create", &scoped), false)
}

fn psession_run(args: &Value, guard: &Guard) -> (Value, bool) {
    if let Err(e) = guard.deny_if_read_only("psession_run") {
        return (json!({ "error": e }), true);
    }
    (psession::execute("psession_run", args), false)
}

// --- http -------------------------------------------------------------------

fn http_request(args: &Value) -> (Value, bool) {
    let url = args.get("url").and_then(|v| v.as_str()).unwrap_or("");
    if url.is_empty() {
        return (json!({ "error": "url is required" }), true);
    }
    let method = args.get("method").and_then(|v| v.as_str()).unwrap_or("GET");
    let headers: HashMap<String, String> = args
        .get("headers")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let body = args.get("body").and_then(|v| v.as_str());
    let timeout_secs = args
        .get("timeout_secs")
        .and_then(|v| v.as_u64())
        .unwrap_or(30);

    let client = match reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return (
                json!({ "error": format!("failed to build client: {e}") }),
                true,
            )
        }
    };

    let start = std::time::Instant::now();
    let mut request = match method.to_uppercase().as_str() {
        "GET" => client.get(url),
        "POST" => client.post(url),
        "PUT" => client.put(url),
        "DELETE" => client.delete(url),
        "PATCH" => client.patch(url),
        "HEAD" => client.head(url),
        other => {
            return (
                json!({ "error": format!("unsupported method: {other}") }),
                true,
            )
        }
    };
    for (k, v) in &headers {
        request = request.header(k.as_str(), v.as_str());
    }
    if let Some(b) = body {
        request = request.body(b.to_string());
    }

    match request.send() {
        Ok(response) => {
            let elapsed = start.elapsed().as_millis() as u64;
            let status = response.status().as_u16();
            let resp_headers: HashMap<String, String> = response
                .headers()
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
                .collect();
            let text = response.text().unwrap_or_default();
            let len = text.len();
            const BODY_CAP: usize = 500_000;
            (
                json!({
                    "success": (200..300).contains(&status),
                    "status_code": status,
                    "headers": resp_headers,
                    "body": if len > BODY_CAP { &text[..BODY_CAP] } else { &text },
                    "body_length": len,
                    "truncated": len > BODY_CAP,
                    "response_time_ms": elapsed
                }),
                false,
            )
        }
        Err(e) => (json!({ "error": format!("request failed: {e}") }), true),
    }
}
