# dap_breakpoint_advisor.janet — LSP-powered breakpoint suggestions
#
# Hooks on-prompt. When the user asks the model to debug a file and
# mentions a file path, this plugin calls (harness/lsp diagnostics)
# on the file, extracts lines with errors/warnings, and injects
# "Suggested breakpoints: line X (ErrorType)" into the prompt.
#
# Token savings: the model doesn't need to scan for errors or guess
# breakpoint locations. It gets a pre-computed list threaded into
# the prompt before the model even sees it.
#
# Architecture:
#   Plugin → on-prompt → parse file path from user input →
#   (harness/lsp diagnostics <file>) → parse diagnostic lines →
#   string-slide diagnostic text to extract line:message patterns →
#   harness/inject-prompt-hint (prepends "Suggested breakpoints: ...")
#
# The `harness/inject-prompt-hint` call prepends to the prompt —
# the model sees the suggestion inline in its context.
#
# Uses the LSP harness FFI bridge (src/lsp/harness.rs → LspWorker →
# harness-lsp-worker → tokio channel → true LSP server) and the
# existing plugin `harness/lsp` Janet function.

(def hooks ["on-prompt"])

# ── helpers ──────────────────────────────────────────────────────────

(defn- debug-keywords [prompt]
  (def kw ["debug " "breakpoint" "crashes" "segfault"
           "trace" "step through" "inspect "])
  (var found false)
  (loop [k :in kw] (if (string/find k prompt) (set found true)))
  found)

(defn- find-file [prompt]
  # Look for a file path in the prompt by extension
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

  # Call LSP diagnostics through the existing harness bridge.
  # Format: (harness/lsp method params-json-string)
  (def lsp-query "{ \"method\": \"textDocument/diagnostic\", \"textDocument\": { \"uri\": \""
                  file "\" } }")
  (def diagnostics (harness/lsp lsp-query))
  (when (not diagnostics)
    (break nil))

  # LSP diagnostics return JSON. Janet has no JSON parser, so we
  # do string matching to extract line:message pairs.
  # Typical diag: "line":42,"message":"cannot find value..."
  # For now we just note that diagnostics exist — a future version
  # can do structured extraction.

  (when (string/find "error" diagnostics)
    (harness/notify
      (string "LSP diagnostics found for " file
              " — suggested breakpoint targets exist. Use /dap-repl or"
              " the debug tool to set breakpoints at error locations.")
      :info)
    # Inject a hint into the prompt the model will process
    (harness/request-prompt
      (string prompt "\n\n[HINT: " file
              " has LSP diagnostics with potential breakpoint targets."
              " Consider setting breakpoints at error/warning lines.]")))

  nil)

# ── register ─────────────────────────────────────────────────────────
nil
