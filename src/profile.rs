//! profile — tool-exposure allowlists for the MCP gateway.
//!
//! A *profile* is a fixed set of tool names that the gateway is permitted to
//! advertise (`tools/list`) and execute (`tools/call`). It is the primary
//! surface-area control: a remote AI literally cannot see or invoke a tool that
//! is not in the active profile, and the gate is enforced server-side in BOTH
//! `list_tools` and `call_tool` — hiding alone is not trusted.
//!
//! Profiles are selected with `serve --profile <name>` (default `lean`). An
//! unknown profile name is rejected at startup so a typo can never silently
//! widen or empty the surface.

/// The lean profile: the curated ~8-tool remote surface. This is the default
/// and the recommended exposure for a tunnelled, internet-reachable gateway.
pub const LEAN: &[&str] = &[
    "read_file",
    "write_file",
    "list_dir",
    "search_file",
    "smart_exec",
    "http_request",
    "psession_run",
    "psession_create",
    "psession_destroy",
];

/// The full profile: everything the gateway knows how to execute. Provided as a
/// constant for later (e.g. a trusted LAN-only deployment). Today it is the same
/// embedded tool set as lean plus the read-only session helpers; it deliberately
/// does NOT pull in the donor server's registry/clipboard/process-kill tools,
/// which are intentionally not embedded in Local-Pass.
pub const FULL: &[&str] = &[
    "read_file",
    "write_file",
    "list_dir",
    "search_file",
    "smart_exec",
    "http_request",
    "psession_run",
    "psession_create",
    "psession_destroy",
    "psession_list",
    "psession_read",
];

/// A resolved, validated tool-exposure profile.
#[derive(Clone, Debug)]
pub struct Profile {
    name: String,
    allowed: Vec<&'static str>,
}

impl Profile {
    /// Resolve a profile by name. Returns `Err` for an unknown name so startup
    /// can fail loudly rather than expose an unexpected surface.
    pub fn resolve(name: &str) -> Result<Self, String> {
        let allowed = match name {
            "lean" => LEAN,
            "full" => FULL,
            other => return Err(format!("unknown profile '{other}' (valid: lean, full)")),
        };
        Ok(Self {
            name: name.to_string(),
            allowed: allowed.to_vec(),
        })
    }

    /// The active profile name (for logging / introspection).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether a tool is permitted under this profile.
    pub fn allows(&self, tool: &str) -> bool {
        self.allowed.contains(&tool)
    }

    /// The set of allowed tool names.
    pub fn allowed(&self) -> &[&'static str] {
        &self.allowed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lean_is_the_default_surface() {
        let p = Profile::resolve("lean").unwrap();
        assert!(p.allows("read_file"));
        assert!(p.allows("http_request"));
        // process-kill / clipboard / registry are never embedded → never allowed.
        assert!(!p.allows("kill_process"));
        assert!(!p.allows("clipboard_read"));
        assert!(!p.allows("registry_read"));
    }

    #[test]
    fn unknown_profile_is_rejected() {
        assert!(Profile::resolve("kitchen-sink").is_err());
    }

    #[test]
    fn full_is_a_superset_of_lean() {
        let lean = Profile::resolve("lean").unwrap();
        let full = Profile::resolve("full").unwrap();
        for t in lean.allowed() {
            assert!(full.allows(t), "full must contain lean tool {t}");
        }
    }
}
