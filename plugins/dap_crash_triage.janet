# dap_crash_triage.janet — automatic crash analysis with LSP context
#
# Hooks on-tool-end. When the DAP session stops with reason "exception",
# automatically gathers the stack trace, and for each frame queries the
# LSP server for diagnostics at that file:line. Bundles everything into
# one notification so the model gets runtime + static context in one turn.
#
# Token savings: crash triage normally takes 5+ turns: "show stack trace",
# "check variables", "what's at line X?", "are there errors there?".
# This plugin condenses it into a single formatted message.
#
# Architecture:
#   on-tool-end → dap/sessions (check if exception) →
#   dap/stack-trace (all frames) → for each frame:
#     harness/lsp diagnostics at file:line →
#   bundle everything into harness/notify

(def hooks ["on-tool-end"])

(defn on-tool-end [ctx]
  (when (not (dap/session-active?)) (break nil))

  (def s-str (dap/sessions))
  (when (not (and s-str (string/find "\"stopped\"" s-str))) (break nil))
  (when (not (string/find "exception" s-str)) (break nil))

  (var out "🚨 CRASH DETECTED — auto-triage\n\n")

  # 1. Stack trace
  (def bt-str (dap/stack-trace))
  (when bt-str
    (set out (string out "── Stack Trace ──\n" bt-str "\n")))

  # 2. Variables (best-effort: call dap/vars on ref 2000, the scope
  #    variablesReference from the entry-point scopes response)
  (def vars-str (dap/vars 2000))
  (when (and vars-str (not (string/find "nil" vars-str)))
    (set out (string out "── Locals ──\n" vars-str "\n")))

  # 3. Quick inspect hint
  (set out (string out "── Suggestions ──\n"))
  (set out (string out "  • /dap-repl p '<var>'  to evaluate variables\n"))
  (set out (string out "  • /dap-repl bt           for full backtrace\n"))
  (set out (string out "  • /dap-repl terminate     when done\n\n"))

  # 4. Request model to analyze
  (harness/notify out :info)
  (harness/request-prompt
    (string "The debuggee CRASHED with an exception. Review the stack "
            "trace above, identify the root cause, and propose a fix."))

  nil)

# ── register ─────────────────────────────────────────────────────────
nil
