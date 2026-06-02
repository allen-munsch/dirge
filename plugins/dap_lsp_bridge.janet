# dap_lsp_bridge.janet — shared DAP↔LSP state cache
#
# Caches DAP session state in a shared Janet table. Other plugins
# (dap_breakpoint_advisor, dap_crash_triage, dap_profiler_crossref)
# read from this cache instead of making redundant FFI calls.
#
# This plugin is DAP-only — it does NOT call harness/lsp directly.
# LSP-aware plugins that consume this cache must do their own
# (harness/lsp?) guard. See project memory:
#   "Always guard LSP-dependent plugin code with
#    (when (dyn :lsp-available) ...)"
#
# Architecture:
#   on-tool-end → if session active:
#     (dap/sessions) → parse stop_reason, thread_id →
#     cache in dap_lsp_cache table
#
# Provides:
#   (dap-lsp/last-stop-reason)  → :entry | :breakpoint | :step | :exception
#   (dap-lsp/last-thread-id)    → integer or 1

(def hooks ["on-tool-end"])

# ── shared cache ─────────────────────────────────────────────────────
(dyn :dap-lsp-cache @{})

# ── hook: refresh cache after every tool call ────────────────────────

(defn on-tool-end [ctx]
  (when (not (dap/session-active?))
    (break nil))

  (def cache (dyn :dap-lsp-cache))

  # Cache session summary
  (def s-str (dap/sessions))
  (when s-str
    (cond
      (string/find "\"entry\"" s-str)
        (put cache :stop-reason :entry)

      (string/find "\"breakpoint\"" s-str)
        (put cache :stop-reason :breakpoint)

      (string/find "\"step\"" s-str)
        (put cache :stop-reason :step)

      (string/find "\"exception\"" s-str)
        (put cache :stop-reason :exception)

      (put cache :stop-reason :unknown)))

  # Cache thread ID
  (when s-str
    (def tid-start (string/find "\"thread_id\": " s-str))
    (when tid-start
      (set tid-start (+ tid-start 14))
      (var tid-end tid-start)
      (while (and (< tid-end (length s-str))
                  (>= (get s-str tid-end) 48)
                  (<= (get s-str tid-end) 57))
        (set tid-end (+ tid-end 1)))
      (def tid-str (string/slice s-str tid-start tid-end))
      (when (not (empty? tid-str))
        (put cache :thread-id (math/parse-int tid-str)))))

  nil)

# ── public API — usable by other Janet plugins ───────────────────────

(defn dap-lsp/last-stop-reason []
  (def cache (dyn :dap-lsp-cache))
  (get cache :stop-reason :unknown))

(defn dap-lsp/last-thread-id []
  (def cache (dyn :dap-lsp-cache))
  (get cache :thread-id 1))

# ── register ─────────────────────────────────────────────────────────
nil
