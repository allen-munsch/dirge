# dap-toolkit — LLM-callable tool that layers structured debugging on DAP
#
# Architecture: this is a Janet plugin that registers an LLM-callable tool
# (`dap_toolkit`) running inside dirge's plugin host (`src/plugin/`).
# It does NOT call into DAP directly — there are no Janet↔DapSessionManager
# FFI bindings yet. Instead, it returns structured recipes the agent follows
# using the `debug` tool (`src/agent/tools/debug.rs`), which dispatches to
# `DapSessionManager` (`src/dap/session.rs`).
#
# The `debug` tool talks to real adapters via `DapClient::spawn_stdio`
# (`src/dap/client.rs`), which spawns adapter processes isolated by
# `setsid()` (own session, no controlling terminal). Adapter auto-detection
# is handled by `src/dap/config.rs` using `defaults.json` and `which::which`.
#
# DAP message framing (Content-Length headers) is in `src/dap/framing.rs`.
# DAP protocol types are in `src/dap/types.rs` — a thin compatibility shim
# over the `dap` crate with local argument structs carrying `extra: Value`
# for adapter-specific extensions.
#
# Plugin tool registration: `harness/register-tool` (P9a API, mirroring
# pi's `api.registerTool`). The handler receives raw JSON-string args from
# the LLM. Janet's bundled runtime has no JSON decoder, so we do string
# matching against scenario keywords. For structured parsing, a plugin
# would need to bundle a JSON library or wait for Janet FFI bindings.
#
# The handler returns a JSON string containing instructions the agent
# follows. This creates a meta-tool: the agent calls dap_toolkit to
# discover *how* to use the debug tool effectively, then calls the
# debug tool with the recipe's steps.
#
# Execution mode: this tool doesn't declare a mode — it inherits the
# default (Parallel). The recipes it returns are read-only guidance;
# the actual debug tool calls (in `src/agent/tools/debug.rs`) are
# registered as Sequential in `src/agent/builder/loop_tools.rs:436`
# because launch/attach mutate session state and spawn subprocesses.

(def hooks [])

(defn- dap-toolkit-handler [args]
  # args is the raw JSON string from the LLM, e.g. {"scenario": "crash"}.
  # Janet's bundled runtime has no JSON decoder — we do string matching.
  # For production use, bundle a Janet JSON library or add FFI bindings
  # to serde_json via the Janet plugin host.
  (def action
    (if (string/find "crash" args) :crash
      (if (string/find "watch" args) :watch
        (if (string/find "profile" args) :profile
          (if (string/find "attach" args) :attach
            (if (string/find "step" args) :step
              :generic))))))

  # Each recipe references specific `debug` tool actions. These map to
  # DapSessionManager methods in src/dap/session.rs:
  #   launch → launch() / launch_with_client()
  #   set_breakpoints → set_breakpoints()   / set_breakpoints request
  #   continue → continue_()               / continue request
  #   stack_trace → stack_trace()           / stackTrace request
  #   scopes → scopes()                     / scopes request
  #   variables → variables()               / variables request
  #   evaluate → evaluate()                 / evaluate request
  #   step_over → step_over()               / next request
  #   terminate → terminate()               / terminate request
  # Each method goes through DapClient::request/notify (src/dap/client.rs),
  # which uses Content-Length framing (src/dap/framing.rs).
  (case action
    :crash
    "{ \"recipe\": \"crash-investigation\", \"steps\": [
      \"debug launch { program: <binary>, stop_on_entry: true }\",
      \"debug set_breakpoints { file: <source>, line: <line> } on the line BEFORE the crash\",
      \"debug continue — run to the suspect code\",
      \"debug stack_trace { thread_id: 1, levels: 10 } — capture the full call chain\",
      \"debug scopes { frame_id: 0 } — get the scope for the top frame\",
      \"debug variables { variable_ref: <ref> } — inspect every suspicious variable\",
      \"debug evaluate { expression: '<var>', frame_id: 0 } — check pointer values for null\",
      \"Report: the exact crashing line, the null/dangling value, and the fix\"
    ]}"

    :watch
    "{ \"recipe\": \"watchpoint-workflow\", \"steps\": [
      \"debug launch { program: <file>, stop_on_entry: true }\",
      \"debug set_breakpoints { file: <file>, line: <line>, condition: '<expr>' }\",
      \"debug continue — each stop, evaluate the watched expression\",
      \"debug evaluate { expression: '<watch-expr>', frame_id: <id> }\",
      \"Repeat continue+eval until the watched value changes unexpectedly\"
    ]}"

    :profile
    "{ \"recipe\": \"sampling-profiler\", \"steps\": [
      \"debug launch { program: <binary>, stop_on_entry: true }\",
      \"Set breakpoints at the top 5 functions by suspected cost\",
      \"Run debug continue 20 times, capturing stack_trace each stop\",
      \"Aggregate: which functions appear most often in the samples?\",
      \"That's your hot path — optimize those first\"
    ]}"

    :attach
    "{ \"recipe\": \"live-attach\", \"steps\": [
      \"First, find the PID: run `ps aux | grep <process>`\",
      \"debug attach { pid: <pid> }\",
      \"debug threads — see all running threads\",
      \"debug stack_trace { thread_id: <id> } — inspect each thread\",
      \"debug pause — freeze execution\",
      \"debug evaluate — inspect live state without killing the process\",
      \"debug continue — resume when done\"
    ]}"

    :step
    "{ \"recipe\": \"step-through\", \"steps\": [
      \"debug launch { program: <file>, stop_on_entry: true }\",
      \"debug set_breakpoints { file: <file>, line: <entry-line> }\",
      \"debug continue\",
      \"For each line of interest: debug step_over or debug step_in\",
      \"After each step: debug evaluate { expression: '<var>' } to watch variables change\",
      \"This gives you a frame-by-frame trace of value changes\"
    ]}"

    "{ \"recipe\": \"generic-debug\", \"steps\": [
      \"First: identify the file and approximate line of the bug\",
      \"debug launch { program: <file>, stop_on_entry: true }\",
      \"debug set_breakpoints { file: <file>, line: <line> }\",
      \"debug continue\",
      \"debug stack_trace { thread_id: 1, levels: 5 }\",
      \"debug evaluate { expression: '<key-variable>' }\",
      \"Iterate: set more breakpoints, step through, narrow down\"
    ]}"))

# Register the tool with dirge's plugin host (src/plugin/extension.rs).
# Parameters: name, description, label, JSON-schema, handler-fn-name.
# The host wraps this in a JanetLoopTool adapter (src/plugin/extension.rs)
# and pushes it into the LoopTool registry via build_loop_tools
# (src/agent/builder/loop_tools.rs:458-484).
(harness/register-tool
  "dap_toolkit"
  "Get a structured debugging recipe for specific scenarios. Call this BEFORE using the debug tool to get step-by-step guidance. Args: { scenario: 'crash' | 'watch' | 'profile' | 'attach' | 'step' | 'generic' }"
  "DAP Toolkit"
  "{\"type\":\"object\",\"properties\":{\"scenario\":{\"type\":\"string\",\"enum\":[\"crash\",\"watch\",\"profile\",\"attach\",\"step\",\"generic\"]}},\"required\":[\"scenario\"]}"
  "dap-toolkit-handler")
