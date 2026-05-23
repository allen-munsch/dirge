pub mod client;
pub mod config;
pub mod tool;

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;
use tool::McpTool;

use crate::permission::ask::AskSender;
use crate::permission::checker::PermCheck;

pub struct McpClientManager {
    /// Connection state per server, by name. Each `Arc<SharedConnection>`
    /// is the SINGLE owner of its peer + RunningService. Cloned into every
    /// `McpTool` from that server so manual `/mcp reconnect` AND tool-side
    /// auto-reconnect share the same swap target (M-R1 + M-R4 fix).
    connections: HashMap<String, Arc<client::SharedConnection>>,
    /// Per-server reconnect serializer + generation counter. Cloned into
    /// every `McpTool` from that server so concurrent failures dedup
    /// across the whole agent — and survive `collect_tools` being
    /// called multiple times during a session (M-R2 fix).
    reconnect_locks: HashMap<String, Arc<Mutex<u64>>>,
    /// Original configs retained so a disconnected server can be
    /// reconnected later via [`reconnect`] (manual `/mcp reconnect`) OR
    /// the tool-side auto-reconnect path (audit H15).
    configs: HashMap<String, config::McpServerConfig>,
}

impl McpClientManager {
    pub async fn connect_all(configs: &HashMap<String, config::McpServerConfig>) -> Self {
        let mut connections = HashMap::new();
        let mut reconnect_locks = HashMap::new();
        for (name, cfg) in configs {
            match client::connect(name.clone(), cfg).await {
                Ok(conn) => {
                    tracing::info!("Connected to MCP server '{}'", name);
                    connections.insert(name.clone(), conn);
                    reconnect_locks.insert(name.clone(), Arc::new(Mutex::new(0u64)));
                }
                Err(e) => {
                    // ALSO emit to stderr so users running without
                    // RUST_LOG / --verbose see that an MCP server
                    // failed to register. Without this, configured
                    // tools just silently never appear and the user
                    // has no idea why.
                    tracing::warn!("Failed to connect to MCP server '{}': {e}", name);
                    eprintln!(
                        "warning: MCP server '{}' failed to connect: {}; its tools won't be available this session",
                        name, e,
                    );
                }
            }
        }
        Self {
            connections,
            reconnect_locks,
            configs: configs.clone(),
        }
    }

    /// Reconnect a single MCP server by name using its original config.
    /// Updates the existing `SharedConnection` in place via `replace`,
    /// so every `McpTool` clone from that server picks up the new
    /// transport on its next call.
    ///
    /// Wired by `/mcp reconnect <name>` (UI slash) for the manual case.
    /// `McpTool` self-reconnects on its own via the same swap path
    /// on transport-class failures.
    #[allow(dead_code)]
    pub async fn reconnect(&mut self, name: &str) -> anyhow::Result<()> {
        let cfg = self.configs.get(name).cloned().ok_or_else(|| {
            anyhow::anyhow!("no config for MCP server '{name}' — was it registered at startup?")
        })?;
        let conn = self.connections.get(name).cloned();

        let (new_peer, new_rs) = client::raw_connect(name, &cfg)
            .await
            .map_err(|e| anyhow::anyhow!("reconnect to '{name}' failed: {e}"))?;

        if let Some(conn) = conn {
            // Swap into the existing shared container so previously-
            // handed-out McpTool clones see the new peer.
            conn.replace(new_peer, new_rs).await;
        } else {
            // No prior connection (server failed to start originally).
            // Create a fresh shared container + start a fresh
            // reconnect lock.
            let conn = Arc::new(client::SharedConnection::new(
                name.to_string(),
                new_peer,
                new_rs,
            ));
            self.connections.insert(name.to_string(), conn);
            self.reconnect_locks
                .entry(name.to_string())
                .or_insert_with(|| Arc::new(Mutex::new(0u64)));
        }
        Ok(())
    }

    pub async fn collect_tools(
        &self,
        permission: Option<PermCheck>,
        ask_tx: Option<AskSender>,
    ) -> Vec<McpTool> {
        let mut all_tools = Vec::new();
        for (server_name, conn) in &self.connections {
            let cfg = self.configs.get(server_name).cloned().map(Arc::new);
            // Reconnect lock from the manager's persistent map. Cloning
            // the Arc bumps the refcount; every McpTool from this
            // server (across this AND any future collect_tools call)
            // shares one canonical lock + gen counter.
            let reconnect_lock = self
                .reconnect_locks
                .get(server_name)
                .cloned()
                .unwrap_or_else(|| Arc::new(Mutex::new(0u64)));
            match client::list_tools(conn).await {
                Ok(tools) => {
                    for definition in tools {
                        all_tools.push(McpTool {
                            server_name: server_name.clone(),
                            definition,
                            connection: Arc::clone(conn),
                            config: cfg.clone(),
                            reconnect_lock: reconnect_lock.clone(),
                            permission: permission.clone(),
                            ask_tx: ask_tx.clone(),
                        });
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to list tools from MCP server '{}': {e}",
                        server_name,
                    );
                    eprintln!(
                        "warning: MCP server '{}' connected but list_tools failed: {}; \
                         its tools won't be available this session",
                        server_name, e,
                    );
                }
            }
        }
        all_tools
    }

    /// Snapshot the current set of (server_name, shared_connection)
    /// pairs. Cheap — clones an `Arc` per server. Used by the
    /// `/mcp` slash command and the info panel to enumerate the
    /// live connections without holding any lock across the await
    /// points that follow (e.g. `list_tools`).
    pub fn connections_snapshot(&self) -> Vec<(String, Arc<client::SharedConnection>)> {
        self.connections
            .iter()
            .map(|(name, conn)| (name.clone(), Arc::clone(conn)))
            .collect()
    }

    pub async fn shutdown(self) {
        for (name, conn) in self.connections {
            conn.shutdown().await;
            tracing::debug!("Disconnected from MCP server '{}'", name);
        }
    }
}
