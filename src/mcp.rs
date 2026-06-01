//! mcp — the rmcp `ServerHandler` that backs the gateway.
//!
//! Implements the MCP server contract by hand (no `#[tool_router]` macro) so
//! the curated tool set can be filtered by the active [`crate::profile::Profile`]
//! in BOTH `list_tools` and `call_tool`, and so each call passes through the
//! [`crate::guard::Guard`]. Tool execution is blocking (the adapted donor code
//! shells out / does sync IO), so `call_tool` offloads to `spawn_blocking`.

use std::borrow::Cow;
use std::sync::Arc;

use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, Content, ErrorData as McpError, Implementation,
    InitializeResult, JsonObject, ListToolsResult, PaginatedRequestParams, ProtocolVersion,
    ServerCapabilities, Tool,
};
use rmcp::service::RequestContext;
use rmcp::RoleServer;
use serde_json::Value;

use crate::guard::Guard;
use crate::mcp_tools;
use crate::profile::Profile;

/// Shared, cheaply-clonable gateway state handed to every session.
#[derive(Clone)]
pub struct GatewayHandler {
    profile: Arc<Profile>,
    guard: Arc<Guard>,
}

impl GatewayHandler {
    pub fn new(profile: Profile, guard: Guard) -> Self {
        Self {
            profile: Arc::new(profile),
            guard: Arc::new(guard),
        }
    }

    /// Build the `Tool` list filtered to the active profile.
    fn visible_tools(&self) -> Vec<Tool> {
        mcp_tools::all_specs()
            .into_iter()
            .filter(|spec| self.profile.allows(spec.name))
            .map(|spec| {
                let schema: JsonObject = match spec.schema {
                    Value::Object(map) => map,
                    _ => JsonObject::new(),
                };
                Tool::new(
                    Cow::Borrowed(spec.name),
                    Cow::Borrowed(spec.description),
                    Arc::new(schema),
                )
            })
            .collect()
    }
}

impl ServerHandler for GatewayHandler {
    fn get_info(&self) -> InitializeResult {
        // rmcp 1.7 marks InitializeResult / Implementation `#[non_exhaustive]`,
        // so they must be built via constructor + `with_*` setters rather than
        // struct literals.
        let description = format!(
            "Remote MCP gateway exposing the '{}' tool profile, root-scoped to {}{}.",
            self.profile.name(),
            self.guard.root().display(),
            if self.guard.is_read_only() {
                " (read-only)"
            } else {
                ""
            }
        );
        let server_info = Implementation::new("local-pass", env!("CARGO_PKG_VERSION"))
            .with_title("Local-Pass Gateway")
            .with_description(description)
            .with_website_url("https://github.com/AIWander/Local-Pass");

        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_protocol_version(ProtocolVersion::LATEST)
            .with_server_info(server_info)
            .with_instructions(format!(
                "Local-Pass gateway. Active profile: '{}'. All file paths are scoped to {}. \
                 Tools not in the profile are neither listed nor callable.",
                self.profile.name(),
                self.guard.root().display()
            ))
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult::with_all_items(self.visible_tools()))
    }

    /// Expose tool definitions for rmcp's built-in task-support validation.
    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.visible_tools().into_iter().find(|t| t.name == name)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let name = request.name.to_string();

        // Server-side profile enforcement: a denied/unknown tool is an error,
        // never an execution. This is the authoritative gate (not list hiding).
        if !self.profile.allows(&name) {
            return Err(McpError::invalid_params(
                format!(
                    "tool '{name}' is not permitted by the active '{}' profile",
                    self.profile.name()
                ),
                None,
            ));
        }

        let args = Value::Object(request.arguments.unwrap_or_default());
        let guard = self.guard.clone();
        let tool_name = name.clone();

        // Tool executors are blocking (shell / sync IO) → offload off the async
        // runtime to avoid stalling the reactor.
        let (value, is_error) =
            tokio::task::spawn_blocking(move || mcp_tools::execute(&tool_name, &args, &guard))
                .await
                .map_err(|e| McpError::internal_error(format!("tool task panicked: {e}"), None))?;

        let body = match serde_json::to_string_pretty(&value) {
            Ok(s) => s,
            Err(_) => value.to_string(),
        };
        let content = vec![Content::text(body)];
        // rmcp 1.7's CallToolResult is `#[non_exhaustive]`: use the success/error
        // constructors (which set the is_error flag) and then attach the machine-
        // readable JSON to the public `structured_content` field, so the response
        // still carries BOTH the pretty text block and the structured value.
        let mut result = if is_error {
            CallToolResult::error(content)
        } else {
            CallToolResult::success(content)
        };
        result.structured_content = Some(value);
        Ok(result)
    }
}

/// Resolve the safe root from CLI args / env / default, in that precedence.
/// Returned path is not yet canonicalized (the [`Guard`] does that).
pub fn resolve_root(explicit: Option<String>) -> std::path::PathBuf {
    if let Some(p) = explicit {
        return std::path::PathBuf::from(p);
    }
    if let Ok(env_root) = std::env::var("LOCALPASS_ROOT") {
        if !env_root.trim().is_empty() {
            return std::path::PathBuf::from(env_root);
        }
    }
    dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."))
}
