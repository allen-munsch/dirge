//! LSP → MCP memory bridge.
//!
//! When `lsp_memory` is enabled in config (or `DIRGE_LSP_MEMORY=true`),
//! [`LspMemorySidecar`] automatically indexes every successful LSP result
//! into connected MCP memory servers (e.g. weft's MosaicDB via
//! `weft_memory_store`).

use rmcp::model::{CallToolRequestParams, JsonObject};
use rmcp::service::{Peer, RoleClient};
use serde_json::json;

/// Holds MCP peers that expose a memory-store tool. Constructed in
/// [`crate::agent::builder::build_agent_inner`] after MCP servers connect.
#[derive(Clone)]
pub struct LspMemorySidecar {
    /// `(server_name, peer)` for each MCP server that has `weft_memory_store`.
    memory_peers: Vec<(String, Peer<RoleClient>)>,
}

impl LspMemorySidecar {
    /// Create from the already-connected MCP client manager.
    /// Scans all connected servers' tools for `weft_memory_store`.
    pub async fn from_manager(manager: &crate::extras::mcp::McpClientManager) -> Self {
        let mut peers = Vec::new();
        for handle in &manager.handles {
            let peer = handle.peer();
            let server_name = handle.server_name.clone();
            match handle.list_tools().await {
                Ok(tools) => {
                    for t in &tools {
                        if t.name == "weft_memory_store" {
                            peers.push((server_name.clone(), peer.clone()));
                            break;
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "LspMemorySidecar: failed to list tools from '{}': {e}",
                        server_name
                    );
                }
            }
        }
        Self {
            memory_peers: peers,
        }
    }

    /// `true` when at least one memory-store peer was found.
    pub fn is_active(&self) -> bool {
        !self.memory_peers.is_empty()
    }

    /// Index an LSP result into all connected memory servers.
    ///
    /// Fires `weft_memory_store` calls with structured metadata so later
    /// `weft_memory_search` / `mosaic_traverse` can surface the result.
    pub async fn index_result(&self, operation: &str, file_path: &str, result_json: &str) {
        if result_json.is_empty() || result_json == "(no results)" {
            return;
        }

        let content = json!({
            "source": "dirge_lsp",
            "operation": operation,
            "file_path": file_path,
            "result": result_json,
        });

        let arguments: Option<JsonObject> = serde_json::from_value(json!({
            "content": content.to_string(),
            "metadata": {
                "source": "dirge_lsp",
                "operation": operation,
                "file_path": file_path,
            }
        }))
        .unwrap_or_default();

        for (server_name, peer) in &self.memory_peers {
            let params = CallToolRequestParams::new("weft_memory_store")
                .with_arguments(arguments.clone().unwrap_or_default());
            match peer.call_tool(params).await {
                Ok(_) => tracing::debug!("lsp_memory: indexed in '{}'", server_name),
                Err(e) => tracing::warn!("lsp_memory: failed to index in '{}': {e}", server_name),
            }
        }
    }
}

/// For use when `lsp_memory` is disabled — all calls are no-ops.
impl Default for LspMemorySidecar {
    fn default() -> Self {
        Self {
            memory_peers: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_sidecar_is_not_active() {
        let sidecar = LspMemorySidecar::default();
        assert!(!sidecar.is_active());
    }

    #[tokio::test]
    async fn default_sidecar_index_result_is_noop() {
        let sidecar = LspMemorySidecar::default();
        // Must not panic — just a no-op when no peers are registered.
        sidecar
            .index_result("definition", "src/main.rs", r#"{"key":"value"}"#)
            .await;
    }

    #[test]
    fn sidecar_send_sync() {
        fn assert_send<T: Send>() {}
        fn assert_sync<T: Sync>() {}
        assert_send::<LspMemorySidecar>();
        assert_sync::<LspMemorySidecar>();
    }
}
