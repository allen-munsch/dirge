use std::collections::HashMap;

use serde::Deserialize;

/// Per-server MCP configuration. Either a stdio command (`command` +
/// `args` + `env`) or a remote URL (`url` + `headers`).
///
/// Both variants accept `allow_external_paths: bool` (default `false`):
/// when set, MCP tool calls from this server bypass the cwd-external-
/// path guard. Other permission rules (the `mcp_tool` rule table,
/// prompt `deny_tools`, doom-loop detection, etc.) still apply — this
/// flag ONLY toggles the path-outside-cwd refusal for tools whose JSON
/// arguments name absolute or relative paths that resolve outside the
/// working directory. Intended for semantic indexers, project-wide
/// search tools, or any MCP server whose legitimate scope is broader
/// than the current project.
///
/// The `Url` variant supports Google and OAuth authentication through
/// The `Url` variant supports Google authentication. When
/// `auth_provider_type` is set, dirge will
/// negotiate tokens before connecting and include them in request headers.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum McpServerConfig {
    Command {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: HashMap<String, String>,
        #[serde(default)]
        allow_external_paths: bool,
    },
    Url {
        url: String,
        #[serde(default)]
        headers: HashMap<String, String>,
        #[serde(default)]
        allow_external_paths: bool,
        /// Auth provider type: `google_credentials`, `service_account_impersonation`,
        /// or `dynamic_discovery`.
        #[serde(default)]
        auth_provider_type: Option<String>,
        /// OAuth PKCE configuration (for `dynamic_discovery` provider).
        #[serde(default)]
        oauth: Option<String>,
        /// OAuth target audience (CLIENT_ID.apps.googleusercontent.com).
        #[serde(default)]
        target_audience: Option<String>,
        /// Service account email to impersonate.
        #[serde(default)]
        target_service_account: Option<String>,
    },
}

impl McpServerConfig {
    /// Whether this server is configured to bypass the cwd-external-
    /// path guard. Defaults to `false` for both variants.
    pub fn allow_external_paths(&self) -> bool {
        match self {
            McpServerConfig::Command {
                allow_external_paths,
                ..
            }
            | McpServerConfig::Url {
                allow_external_paths,
                ..
            } => *allow_external_paths,
        }
    }


}
