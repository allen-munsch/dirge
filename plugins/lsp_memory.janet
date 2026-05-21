# dirge LSP → Memory bridge plugin
# Records LSP results as session entries for observability.
# Complements the Rust LspMemorySidecar feature which handles
# the actual MCP indexing to weft MosaicDB.

(def hooks ["on-tool-end" "on-init"])

# Track which operations we see so the user can observe coverage
(var ops-seen @{})

(defn lsp-on-init [ctx]
  (set ops-seen @{})
  (harness/register-renderer "lsp_result" "lsp-render-result")
  (harness/log "lsp_memory plugin loaded — LSP results recorded as session entries"))

(defn lsp-on-tool-end [ctx]
  (let [tool (ctx :tool)
        output (ctx :output)]
    (when (and (= tool "lsp")
               output
               (not (string/find "no results" output))
               (not (string/find "failed to serialize" output)))
      (try
        (do
          # Store the raw LSP result as a session entry
          (harness/append-entry "lsp_result" output true)
          nil)
        ([err fib]
          (harness/log (string "lsp_memory: store failed: " err))))))
  nil)

# Renderer for lsp_result entries. Shows a truncated preview.
(defn lsp-render-result [data]
  (let [preview (if (> (length data) 120)
                  (string (string/slice data 0 120) "...")
                  data)]
    (harness/render :cyan "LSP result")
    # Replace newlines in the preview so it stays on one line
    (harness/render :white
      (string/replace-all preview "\n" " "))))
