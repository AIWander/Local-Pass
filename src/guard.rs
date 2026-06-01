//! guard — runtime safety rails for embedded tools.
//!
//! Two independent guards, both enforced server-side before a tool runs:
//!
//! 1. **File-root scope.** Every filesystem tool (`read_file`, `write_file`,
//!    `list_dir`, `search_file`) must resolve its target *inside* a configured
//!    safe root. The root comes from `--root <path>`, else `LOCALPASS_ROOT`,
//!    else the user's home dir. Any path that escapes the root (via `..`,
//!    absolute paths, or — for existing paths — symlinks) is rejected before
//!    the operation runs. This is the blast-radius limiter for a tunnelled,
//!    internet-reachable gateway.
//!
//! 2. **Read-only mode.** `--read-only` denies mutating tools (`write_file`)
//!    and all shell execution (`smart_exec`, `psession_*`) regardless of
//!    profile.
//!
//! Guard rejections are returned to the model as tool errors, not silent
//! no-ops, so the caller knows the boundary exists.

use std::path::{Component, Path, PathBuf};

/// Resolved guard configuration, shared (read-only) across all tool calls.
#[derive(Clone, Debug)]
pub struct Guard {
    root: PathBuf,
    read_only: bool,
}

impl Guard {
    /// Build a guard from an explicit root (already resolved by the caller) and
    /// the read-only flag. The root is canonicalized if it exists so later
    /// containment checks compare real paths; if it does not yet exist we keep
    /// the lexical form (it will be created lazily by writes).
    pub fn new(root: PathBuf, read_only: bool) -> Self {
        let root = std::fs::canonicalize(&root).unwrap_or(root);
        Self { root, read_only }
    }

    /// The active safe root (for logging).
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Whether the gateway is in read-only mode.
    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// Reject mutating operations in read-only mode. `op` is a human label used
    /// in the error message (e.g. "write_file", "shell execution").
    pub fn deny_if_read_only(&self, op: &str) -> Result<(), String> {
        if self.read_only {
            Err(format!(
                "{op} is denied: server is running in --read-only mode"
            ))
        } else {
            Ok(())
        }
    }

    /// Resolve a caller-supplied path and confirm it stays inside the safe root.
    ///
    /// Returns the resolved absolute path on success. Works for paths that do
    /// not yet exist (needed for `write_file`): we normalize lexically against
    /// the root, collapsing `.`/`..` *without* touching the filesystem, then
    /// verify the result is still under the root. For paths that DO exist we
    /// additionally canonicalize to defeat symlink escapes.
    pub fn resolve_in_root(&self, raw: &str) -> Result<PathBuf, String> {
        if raw.trim().is_empty() {
            return Err("path is empty".to_string());
        }

        let requested = Path::new(raw);
        // A relative path is interpreted relative to the safe root, never the
        // process CWD — the remote client should not be able to reach the CWD.
        let joined = if requested.is_absolute() {
            requested.to_path_buf()
        } else {
            self.root.join(requested)
        };

        let normalized = lexical_normalize(&joined);

        // If the concrete target exists, canonicalize it (resolves symlinks) and
        // check containment against the canonical root. Otherwise fall back to
        // the lexical form — its nearest existing ancestor is still validated by
        // the lexical containment check below.
        let candidate = std::fs::canonicalize(&normalized).unwrap_or(normalized);

        if !candidate.starts_with(&self.root) {
            return Err(format!(
                "path '{raw}' resolves to '{}', which is outside the safe root '{}'",
                candidate.display(),
                self.root.display()
            ));
        }
        Ok(candidate)
    }
}

/// Collapse `.` and `..` components lexically (no filesystem access).
/// A leading `..` that would climb above the path's anchor is dropped, so the
/// result can never reference a parent of the root once joined under it.
fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    // Nothing to pop (we're at/above the anchor) — ignore, which
                    // prevents climbing above the root.
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guard_at(dir: &Path) -> Guard {
        Guard::new(dir.to_path_buf(), false)
    }

    #[test]
    fn in_root_paths_resolve() {
        let tmp = std::env::temp_dir().join(format!("lp-guard-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp).unwrap();
        let g = guard_at(&tmp);
        let p = g.resolve_in_root("sub/file.txt").unwrap();
        assert!(p.starts_with(g.root()));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn dotdot_escape_is_rejected() {
        let tmp = std::env::temp_dir().join(format!("lp-guard-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp).unwrap();
        let g = guard_at(&tmp);
        // Enough `..` segments to climb above the (deep) temp root must NOT
        // resolve to a path under it — the guard rejects the escape outright.
        assert!(
            g.resolve_in_root("../../../../../../../../etc/passwd").is_err(),
            "deep ../ escape must be rejected, not silently resolved"
        );
        // A `..` that stays within the root is fine.
        let inside = g.resolve_in_root("sub/../allowed.txt").unwrap();
        assert!(inside.starts_with(g.root()));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn absolute_outside_root_is_rejected() {
        let tmp = std::env::temp_dir().join(format!("lp-guard-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp).unwrap();
        let g = guard_at(&tmp);
        let other = if cfg!(windows) {
            "C:\\Windows\\System32\\drivers\\etc\\hosts"
        } else {
            "/etc/passwd"
        };
        assert!(g.resolve_in_root(other).is_err());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn read_only_blocks_mutations() {
        let g = Guard::new(std::env::temp_dir(), true);
        assert!(g.deny_if_read_only("write_file").is_err());
        assert!(g.is_read_only());
    }
}
