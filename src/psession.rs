//! psession — persistent PowerShell sessions that survive across MCP calls.
//!
//! Adapted from the donor `local` server's `psession.rs`, trimmed to the
//! PowerShell backend (the donor WSL path is not exposed by Local-Pass) and
//! kept synchronous. A session is a long-lived `powershell -Command -` child
//! whose stdout/stderr are drained on a background thread into a shared buffer;
//! `psession_run` writes a command plus a unique completion marker and reads
//! back everything up to that marker. State (cwd, env, variables) persists
//! between calls. Root-scoping of the optional `cwd` is applied by the caller
//! (`crate::mcp_tools::psession_create`) before this module sees it.

use once_cell::sync::Lazy;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

#[cfg(windows)]
use std::os::windows::process::CommandExt;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

static PSESSIONS: Lazy<Arc<Mutex<HashMap<String, PersistentSession>>>> =
    Lazy::new(|| Arc::new(Mutex::new(HashMap::new())));

struct PersistentSession {
    name: String,
    child: Child,
    output_buffer: Arc<Mutex<Vec<String>>>,
    history: Vec<String>,
    created_at: String,
}

fn start_reader(stream: impl std::io::Read + Send + 'static, buffer: Arc<Mutex<Vec<String>>>) {
    thread::spawn(move || {
        let reader = BufReader::new(stream);
        for line in reader.lines().map_while(Result::ok) {
            if let Ok(mut buf) = buffer.lock() {
                buf.push(line);
            }
        }
    });
}

/// Dispatch a psession tool by name. Unknown names return an error value.
pub fn execute(name: &str, args: &Value) -> Value {
    match name {
        "psession_create" => psession_create(args),
        "psession_run" => psession_run(args),
        "psession_destroy" => psession_destroy(args),
        "psession_list" => psession_list(args),
        "psession_read" => psession_read(args),
        other => json!({ "error": format!("unknown psession tool: {other}") }),
    }
}

fn psession_create(args: &Value) -> Value {
    let name = args
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("default");
    let cwd = args
        .get("cwd")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let mut cmd = Command::new("powershell");
    cmd.args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", "-"]);
    if let Some(dir) = &cwd {
        cmd.current_dir(dir);
    }
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return json!({ "error": format!("failed to spawn powershell: {e}") }),
    };
    let stdout = match child.stdout.take() {
        Some(s) => s,
        None => return json!({ "error": "failed to capture stdout" }),
    };
    let buffer = Arc::new(Mutex::new(Vec::new()));
    start_reader(stdout, buffer.clone());
    if let Some(stderr) = child.stderr.take() {
        start_reader(stderr, buffer.clone());
    }
    thread::sleep(std::time::Duration::from_millis(200));

    let session_id = format!("powershell_{name}");
    let created = chrono::Local::now().to_rfc3339();
    if let Ok(mut sessions) = PSESSIONS.lock() {
        sessions.insert(
            session_id.clone(),
            PersistentSession {
                name: name.to_string(),
                child,
                output_buffer: buffer,
                history: Vec::new(),
                created_at: created.clone(),
            },
        );
    }
    json!({ "session_id": session_id, "name": name, "created_at": created })
}

fn psession_run(args: &Value) -> Value {
    let session_id = args
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let command = args.get("command").and_then(|v| v.as_str()).unwrap_or("");
    let timeout_secs = args
        .get("timeout_secs")
        .and_then(|v| v.as_u64())
        .unwrap_or(30);
    if session_id.is_empty() || command.is_empty() {
        return json!({ "error": "session_id and command are required" });
    }

    let mut sessions = match PSESSIONS.lock() {
        Ok(s) => s,
        Err(_) => return json!({ "error": "session store poisoned" }),
    };
    let session = match sessions.get_mut(session_id) {
        Some(s) => s,
        None => return json!({ "error": format!("session not found: {session_id}") }),
    };

    let start_pos = session.output_buffer.lock().map(|b| b.len()).unwrap_or(0);
    let marker = format!(
        "__DONE_{}__",
        uuid::Uuid::new_v4()
            .to_string()
            .get(..8)
            .unwrap_or("00000000")
    );
    let stdin = match session.child.stdin.as_mut() {
        Some(s) => s,
        None => return json!({ "error": "stdin not available" }),
    };
    let full_cmd = format!("{command}\nWrite-Output '{marker}'\n");
    if let Err(e) = stdin.write_all(full_cmd.as_bytes()) {
        return json!({ "error": format!("write failed: {e}") });
    }
    if let Err(e) = stdin.flush() {
        return json!({ "error": format!("flush failed: {e}") });
    }
    session.history.push(command.to_string());

    let buffer = session.output_buffer.clone();
    drop(sessions);

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    let mut output_lines = Vec::new();
    let mut found = false;
    while std::time::Instant::now() <= deadline {
        if let Ok(buf) = buffer.lock() {
            let len = buf.len();
            if len > start_pos {
                for (offset, line) in buf[start_pos..len].iter().enumerate() {
                    if line.contains(&marker) {
                        found = true;
                        output_lines = buf[start_pos..start_pos + offset].to_vec();
                        break;
                    }
                }
                if found {
                    break;
                }
            }
        }
        thread::sleep(std::time::Duration::from_millis(50));
    }
    if !found {
        if let Ok(buf) = buffer.lock() {
            if buf.len() > start_pos {
                output_lines = buf[start_pos..].to_vec();
            }
        }
    }

    json!({
        "session_id": session_id,
        "output": output_lines.join("\n"),
        "lines": output_lines.len(),
        "completed": found,
        "timed_out": !found
    })
}

fn psession_destroy(args: &Value) -> Value {
    let session_id = args
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if session_id.is_empty() {
        return json!({ "error": "session_id is required" });
    }
    let mut sessions = match PSESSIONS.lock() {
        Ok(s) => s,
        Err(_) => return json!({ "error": "session store poisoned" }),
    };
    if let Some(mut session) = sessions.remove(session_id) {
        let _ = session.child.kill();
        json!({ "destroyed": session_id })
    } else {
        json!({ "error": format!("session not found: {session_id}") })
    }
}

fn psession_list(_args: &Value) -> Value {
    let sessions = match PSESSIONS.lock() {
        Ok(s) => s,
        Err(_) => return json!({ "error": "session store poisoned" }),
    };
    let list: Vec<Value> = sessions
        .iter()
        .map(|(id, s)| {
            json!({
                "session_id": id,
                "name": s.name,
                "history_count": s.history.len(),
                "buffer_lines": s.output_buffer.lock().map(|b| b.len()).unwrap_or(0),
                "created_at": s.created_at
            })
        })
        .collect();
    let count = list.len();
    json!({ "sessions": list, "count": count })
}

fn psession_read(args: &Value) -> Value {
    let session_id = args
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let tail = args.get("tail").and_then(|v| v.as_u64()).unwrap_or(20) as usize;
    if session_id.is_empty() {
        return json!({ "error": "session_id is required" });
    }
    let sessions = match PSESSIONS.lock() {
        Ok(s) => s,
        Err(_) => return json!({ "error": "session store poisoned" }),
    };
    let session = match sessions.get(session_id) {
        Some(s) => s,
        None => return json!({ "error": format!("session not found: {session_id}") }),
    };
    let buf = match session.output_buffer.lock() {
        Ok(b) => b,
        Err(_) => return json!({ "error": "buffer poisoned" }),
    };
    let total = buf.len();
    let start = total.saturating_sub(tail);
    json!({
        "session_id": session_id,
        "total_lines": total,
        "tail": buf[start..].join("\n")
    })
}
