# /dap-debug slash command — quick launch with auto-breakpoints
#
# Architecture: this Janet plugin registers a slash command via
# `harness/register-command` (Phase 2 API, same as hello_cmd.janet).
# The handler runs in dirge's plugin host (src/plugin/) and injects
# a prompt via the return value — the caller in src/ui/slash/mod.rs
# writes the returned string as the user's next message.
#
# Does NOT call DAP directly (no Janet↔DapSessionManager FFI bindings yet).
# Instead constructs a prompt telling the agent to use the `debug` tool
# (src/agent/tools/debug.rs), which dispatches to DapSessionManager
# (src/dap/session.rs) on the shared DAP_MANAGER singleton.
#
# Adapter auto-detection: when the agent calls `debug launch` without
# an explicit adapter, src/dap/config.rs:select_launch_adapter() picks
# the right adapter from file extension + root markers. For .py files
# with pyproject.toml/requirements.txt present, debugpy wins.
# For .rs files with Cargo.toml, lldb-dap wins (falls back to gdb).
#
# Process isolation: the adapter runs in its own session via setsid()
# in src/dap/client.rs:spawn_stdio() — no controlling terminal, cannot
# tcsetpgrp() the TUI. The DapProcessGuard sends kill(-pgid, SIGKILL)
# on drop to clean up the adapter's entire process tree (debuggee + children).
#
# Background launch: /debug launch spawns adapter handshake + initial
# stop on tokio::spawn (src/ui/slash/cmd_debug.rs:173) so the TUI
# stays responsive. The debug panel (src/ui/tui/panels.rs debug widget)
# picks up session state from DAP_MANAGER.debug_snapshot() on each
# UI tick.
#
# Usage:
#   /dap-debug <file>               launch with stop-on-entry
#   /dap-debug <file> <line>        launch + set breakpoint at line
#   /dap-debug <file> all           launch + set breakpoints at every function
(def hooks [])

(defn- dap-debug-handler [args]
  (def parts (string/split " " args))
  (def file (get parts 0))
  (if (not file)
    (break "usage: /dap-debug <file> [<line>|all]"))

  (def line (get parts 1))
  # The base prompt tells the agent to call `debug launch`. This maps to
  # DebugTool.call() Action::Launch in src/agent/tools/debug.rs:325,
  # which calls DapSessionManager.launch() → launch_with_client() in
  # src/dap/session.rs:212-266. The adapter is auto-detected from the
  # file extension (config.rs:select_launch_adapter).
  (def base-prompt (string
    "debug launch { program: \"" file "\", stop_on_entry: true }\n"))

  (if (not line)
    base-prompt
    (= line "all")
    (string base-prompt
      "After launch, set breakpoints at:\n"
      "- Every function definition in " file "\n"
      "- Every class `__init__` method\n"
      "Then debug continue and tell me where it stopped.\n"
      "Use the `debug` tool for everything.")
    # Single-line breakpoint: maps to Action::SetBreakpoints in
    # debug.rs:387, which calls DapSessionManager.set_breakpoints()
    # (session.rs:450) → DAP setBreakpoints request → adapter.
    (string base-prompt
      "debug set_breakpoints { file: \"" file "\", line: " line " }\n"
      "debug continue\n"
      "Tell me the stack trace and local variables at the breakpoint.")))

# Register with dirge's plugin system. The handler name is resolved at
# script load time by the plugin host (src/plugin/mod.rs). When the
# user types /dap-debug <args>, the slash command dispatcher in
# src/ui/slash/mod.rs routes to the plugin handler path, which calls
# this function. The return value (a string) is written as the user's
# next message, which the agent then picks up and processes.
(harness/register-command "dap-debug" "dap-debug-handler")
