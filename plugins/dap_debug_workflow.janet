# dap-debug-workflow — structured debug→fix→verify cycle via agent orchestration
#
# Architecture: this Janet plugin runs in dirge's plugin host and hooks into
# the agent lifecycle via `on-prompt`, `on-response`, `on-tool-start`, and
# `on-tool-end` hooks. It does NOT call DAP directly. Instead, it detects
# debug-related user prompts and injects structured prompts via
# `harness/request-prompt` — a Phase 2 API that tells the agent what to do.
#
# The agent then calls the `debug` tool (src/agent/tools/debug.rs), which
# dispatches to DapSessionManager (src/dap/session.rs) → DapClient
# (src/dap/client.rs) → real debug adapters (debugpy, lldb-dap, gdb, etc.).
#
# Phase lifecycle (mirrors TDD red→green→refactor, adapted for debugging):
#   Phase 1: LAUNCH    — idempotent; DapSessionManager.terminate_active()
#                        cleans up any prior session before spawning a new
#                        adapter. Adapter auto-detection via
#                        src/dap/config.rs:select_launch_adapter().
#   Phase 2: REPRODUCE — uses debug continue, stack_trace, scopes, variables,
#                        evaluate, step_over to capture bug evidence.
#   Phase 3: FIX+VERIFY — uses debug terminate (sends terminate + disconnect
#                        DAP requests) then re-launches to confirm.
#
# Hook dispatch: the plugin host (src/plugin/mod.rs) calls hook functions
# by name convention: `dap-debug-workflow-on-<hook-name>`. Each receives a
# Janet table `ctx` with keys like :prompt, :response, :tool, :error.
#
# The `harness/request-prompt` API works like the `workflow.janet` example —
# it injects text that the agent treats as its next instruction, overriding
# the normal model response. This is inversion of control: the plugin drives
# the model, not vice versa.
#
# Known limitation: this plugin does keyword matching on user prompts. It
# cannot introspect the actual DAP session state (no Janet↔DapSessionManager
# FFI bindings yet). When those bindings are added (exposing launch, step,
# evaluate, etc. as Janet functions), the plugin can drive the debugger
# directly instead of prompting the agent.

(def hooks ["on-prompt" "on-response" "on-tool-start" "on-tool-end"])

(var phase :idle)
(var target-file nil)
(var target-bug "")

# ── keyword detection ──────────────────────────────────────────────
# Matches against user prompt text. This is heuristic — a proper
# implementation would parse the prompt for file paths (using the
# same extension→adapter mapping as src/dap/config.rs:get_matching_adapters)
# and validate them against the filesystem.

(defn- debug-keywords [prompt]
  (def kw ["debug " "breakpoint" "crashes" "segfault" "null pointer"
           "race condition" "deadlock" "memory leak" "hangs" "infinite loop"
           "wrong output" "unexpected" "trace" "step through"
           "isn't working" "why is" "broken" "bug" "fails"])
  (var found false)
  (loop [k :in kw] (if (string/find k prompt) (set found true)))
  found)

# ── phase prompts — injected via harness/request-prompt ────────────
# Each prompt maps to specific `debug` tool actions. These actions are
# defined in src/agent/tools/debug.rs and dispatch to DapSessionManager
# methods (src/dap/session.rs). The timeout for each operation defaults
# to 30s, clamped to [5, 300] by clamp_timeout() in debug.rs:214.

(defn- launch-prompt [file]
  (string
    "DEBUG WORKFLOW — Phase 1: LAUNCH " file "\n\n"
    "1. Use debug launch with stop_on_entry: true on " file "\n"
    "2. Set breakpoints at the most likely failure points:\n"
    "   - The function where the bug manifests\n"
    "   - Any input validation or parsing code\n"
    "   - Resource allocation/deallocation points\n"
    "3. debug continue to reproduce the bug\n"
    "4. Report where the program stopped and what you see.\n\n"
    "Use the `debug` tool for everything."))

(defn- reproduce-prompt []
  (string
    "DEBUG WORKFLOW — Phase 2: REPRODUCE\n\n"
    "1. debug continue to run to the first breakpoint\n"
    "2. debug stack_trace to see the call chain\n"
    "3. debug scopes then debug variables to inspect locals\n"
    "4. debug evaluate suspicious expressions\n"
    "5. Step through with debug step_over\n"
    "6. Capture: exact line, variable values, root cause\n\n"
    "Report findings. Do NOT fix anything yet."))

(defn- fix-verify-prompt [bug-desc]
  (string
    "DEBUG WORKFLOW — Phase 3: FIX + VERIFY\n\n"
    "Bug: " bug-desc "\n\n"
    "1. Apply minimal fix using edit/write tool\n"
    "2. debug terminate (restart: true if applicable)\n"
    "3. Re-run: debug launch same file, debug continue\n"
    "4. Verify breakpoint doesn't hit, program completes normally\n\n"
    "Report: fix applied, test result, follow-up."))

# ── hooks ──────────────────────────────────────────────────────────

(defn on-prompt [ctx]
  (let [prompt (ctx :prompt)]
    (if (and (= phase :idle) (debug-keywords prompt))
      (do
        # Extract a file path from the prompt by scanning for known
        # extensions. This mirrors the extension list from
        # src/dap/defaults.json (debugpy: .py, lldb-dap: .rs/.c/.cpp/.zig,
        # dlv: .go, js-debug-adapter: .js/.ts, rdbg: .rb).
        # A production version would read defaults.json and use its
        # file_types arrays, but Janet's bundled runtime has no JSON
        # parser — that requires FFI bindings or a bundled JSON lib.
        (var file nil)
        (each ext [".py" ".rs" ".c" ".cpp" ".go" ".js" ".ts" ".rb"]
          (when (not file)
            (let [idx (string/find ext prompt)]
              (when idx
                (var start idx)
                (while (and (> start 0)
                            (not= (string/slice prompt (dec start) start) " ")
                            (not= (string/slice prompt (dec start) start) "\""))
                  (set start (dec start)))
                (set file (string/slice prompt start (+ idx (length ext))))))))
        (if file
          (do
            (harness/log (string "dap-workflow: debug detected for " file))
            (set target-file file)
            (set target-bug prompt)
            (set phase :launch)
            (harness/request-prompt (launch-prompt file))
            "🔍 Debug workflow started — Phase 1: Launch")
          nil))
      nil)))

(defn on-response [ctx]
  (case phase
    :launch
    (do
      (set phase :reproduce)
      (harness/request-prompt (reproduce-prompt))
      "🔬 Phase 2: Reproduce")

    :reproduce
    (do
      (set phase :fix-verify)
      (harness/request-prompt (fix-verify-prompt (or (ctx :response) "")))
      "🔧 Phase 3: Fix + Verify")

    :fix-verify
    (do
      (set phase :idle)
      (set target-file nil)
      (set target-bug "")
      (harness/log "dap-workflow: debug cycle complete")
      nil)

    nil))

(defn on-tool-start [ctx]
  (when (not= phase :idle)
    (let [tool (ctx :tool)]
      (when tool
        (harness/log (string "dap-workflow [" phase "]: " tool)))))
  nil)

(defn on-tool-end [_ctx] nil)
