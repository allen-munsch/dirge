//! Central output chokepoint for asynchronous messages that need to
//! reach the user but DON'T originate from the agent's own event
//! stream.
//!
//! Previously, off-stream messages (MCP server stderr, plugin
//! warnings, background-task lifecycle pings, etc.) reached the
//! user via inconsistent paths: some went through `tracing::warn!`
//! (which writes to plain stderr and paints over the alt-screen
//! UI), some called `renderer.write_line` directly from inside
//! deeply-nested task spawns (requiring `&mut Renderer` access in
//! places it shouldn't be), and some leaked control bytes through
//! sanitizers built for one specific source.
//!
//! This module owns ONE `tokio::sync::mpsc::UnboundedSender<Notification>`
//! as a process-global; producers send a typed `Notification` and
//! the UI event loop drains the channel with the same
//! `tokio::select!` arm shape as `ask_rx` / `question_rx` /
//! `lifecycle_rx`. The receiver path runs through the standard
//! `Renderer::write_line` pipeline — same wrapping, same theming,
//! same scroll behaviour — so a message from an MCP server reads
//! the same way as an agent error or a permission denial.

use std::sync::OnceLock;

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

/// One off-stream message destined for the chat area. Variants pick
/// the visual treatment (color + prefix); content is plain text
/// that has ALREADY been sanitized of escape sequences by the
/// producer.
#[derive(Debug, Clone)]
pub enum Notification {
    /// Output from an MCP child server's stderr. Renders dim with
    /// a `[mcp:<server>]` prefix.
    McpLog { server: String, line: String },
    /// Generic informational note from a non-agent source (plugins,
    /// background tasks). Renders in the agent color.
    #[allow(dead_code)] // reserved for future producers
    Info(String),
    /// Warning from a non-agent source. Renders in the warn color.
    #[allow(dead_code)]
    Warn(String),
    /// Error from a non-agent source. Renders in the error color.
    #[allow(dead_code)]
    Error(String),
}

/// Global sender. Installed once at UI startup; cloned by every
/// producer (forwarder tasks, plugin hooks, etc.) via `sender()`.
/// `OnceLock` rather than `LazyLock` so producers running BEFORE
/// `install()` get `None` and can quietly fall back (or drop) —
/// during early CLI / config parsing we don't have a UI yet and
/// shouldn't crash.
static TX: OnceLock<UnboundedSender<Notification>> = OnceLock::new();

/// Install the channel and return the receiver. The UI event loop
/// owns the receiver and pulls from it in its `tokio::select!`.
/// Idempotent: second call returns `None` so a test harness that
/// re-enters the UI startup path doesn't double-install.
pub fn install() -> Option<UnboundedReceiver<Notification>> {
    if TX.get().is_some() {
        return None;
    }
    let (tx, rx) = unbounded_channel();
    if TX.set(tx).is_err() {
        return None;
    }
    Some(rx)
}

/// Get a clone of the sender for producers. Returns `None` when the
/// channel hasn't been installed yet (CLI-only paths, tests).
/// Producers should `.ok()`-style the failure — dropping the
/// notification is preferable to crashing in early-startup code.
pub fn sender() -> Option<UnboundedSender<Notification>> {
    TX.get().cloned()
}

/// Send an MCP log line. Convenience wrapper that callers (the
/// stderr forwarder) can use without constructing the enum.
/// Drops silently if the channel isn't installed.
pub fn notify_mcp_log(server: &str, line: &str) {
    if let Some(tx) = sender() {
        let _ = tx.send(Notification::McpLog {
            server: server.to_string(),
            line: line.to_string(),
        });
    }
}
