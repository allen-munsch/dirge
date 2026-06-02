# dap_breakpoint_advisor.janet — LSP-powered breakpoint suggestions
#
# Hooks on-prompt. When the user asks the model to debug a file and
# mentions a file path, this plugin calls (harness/lsp-diagnostics file)
# and injects "suggested breakpoint targets exist at error/warning lines"
# into the prompt so the model doesn't need to scan for errors.
#
# Uses the typed LSP wrapper `harness/lsp-diagnostics` which takes a
# single file-path argument and returns the diagnostic JSON from the
# language server. Guarded by `(harness/lsp?)` — returns nil silently
# when LSP is unavailable (not compiled, no server, disabled at runtime).
#
# Architecture:
#   Plugin → on-prompt → parse file path from user input →
#   (harness/lsp-diagnostics file) [guarded] →
#   if diagnostics found → harness/request-prompt with hint →
#   else nil (silent pass-through)

(def hooks ["on-prompt"])

# ── helpers ──────────────────────────────────────────────────────────

(defn- debug-keywords [prompt]
  (def kw ["debug " "breakpoint" "crashes" "segfault"
           "trace" "step through" "inspect "])
  (var found false)
  (loop [k :in kw] (if (string/find k prompt) (set found true)))
  found)

(defn- find-file [prompt]
  (var file nil)
  (each ext [".py" ".rs" ".c" ".cpp" ".go" ".js" ".ts" ".rb" ".java"]
    (when (not file)
      (let [idx (string/find ext prompt)]
        (when idx
          (var start idx)
          (while (and (> start 0)
                      (not= (string/slice prompt (dec start) start) " ")
                      (not= (string/slice prompt (dec start) start) "\"")
                      (not= (string/slice prompt (dec start) start) "'"))
            (set start (dec start)))
          (set file (string/slice prompt start (+ idx (length ext))))))))
  file)

# ── hook ─────────────────────────────────────────────────────────────

(defn on-prompt [ctx]
  (def prompt (ctx :prompt))
  (when (not (debug-keywords prompt))
    (break nil))

  (def file (find-file prompt))
  (when (not file)
    (break nil))

  # Guard: only call LSP if the bridge is live AND wired to a real
  # language server. Without this guard the C function segfaults
  # because it reads from Janet's argv expecting 5 separate args
  # and our call path doesn't match.
  (when (not (harness/lsp?))
    (break nil))

  (def diagnostics (harness/lsp-diagnostics file))
  (when (not diagnostics)
    (break nil))

  # LSP diagnostics return JSON. Check for error/warning presence
  # via string matching (Janet has no JSON decoder).
  (when (or (string/find "error" diagnostics)
            (string/find "warning" diagnostics))
    (harness/notify
      (string "LSP diagnostics found for " file
              " — suggested breakpoint targets exist")
      :info)
    (harness/request-prompt
      (string prompt "\n\n[HINT: " file
              " has LSP diagnostics with potential breakpoint targets."
              " Consider setting breakpoints at error/warning lines.]")))

  nil)

# ── register ─────────────────────────────────────────────────────────
nil
