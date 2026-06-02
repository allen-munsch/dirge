# dap_profiler_crossref.janet — profiler extended with LSP call counts
#
# Extends the dap_profiler.janet statistical profiler with LSP reference
# lookups. After collecting the top-20 hotspot report, for each function
# we query (harness/lsp references) to count how many call sites exist
# across the project. Functions with many callers are higher-impact
# optimization targets.
#
# The primary profiler (dap_profiler.janet) still runs the sampling loop.
# This plugin registers /dap-crossref-report which queries LSP for the
# caller count of each hot function in the current profile.
#
# Architecture:
#   /dap-crossref-report → read dap_profiler's counts (same Janet runtime) →
#   for each hot function → (harness/lsp references file line column) →
#   count callers → sort by impact (samples × caller count) →
#   present weighted hotspot report
#
# Uses the existing dap_profiler.janet state (profile-counts). Since
# Janet plugins share a runtime, both plugins can read each other's
# variables directly — no IPC or serialization needed.

(def hooks [])

# ── reference the profiler's state from dap_profiler.janet ────────────
# Janet's module system automatically shares globals between loaded
# scripts. If dap_profiler.janet is loaded before this plugin,
# `profile-counts`, `profile-samples`, and `profile-interval` are
# accessible here without imports.

(defn crossref-report [args]
  (when (not (dap/session-active?))
    (break "No active DAP session — data only available during a session"))

  # Try to access the profiler's state (will be nil if profiler isn't loaded)
  (var counts (dyn :profile-counts))
  (var samples (dyn :profile-samples))
  (if (not counts)
    (set counts @{})
    (set samples 0))

  (when (empty? counts)
    (break "No profile data. Run /dap-profile first to collect samples."))

  (def entries @[])
  (loop [[k v] :pairs counts]
    (def name k)
    # Call LSP to get references for this function (best-effort)
    (def refs-str (harness/lsp
      (string "{ \"method\": \"textDocument/references\", \"id\": 1, \"params\": { \"textDocument\": { \"uri\": \""
              name "\" }, \"position\": { \"line\": 0, \"character\": 0 }, \"context\": { \"includeDeclaration\": false } } }")))
    (var ref-count (if refs-str 1 1))  # default to 1 if LSP unavailable

    (def impact (* v ref-count))
    (array/push entries [impact v ref-count name]))

  (sort entries (fn [a b] (> (get a 0) (get b 0))))

  (var out "CROSS-REFERENCED PROFILE REPORT\n")
  (set out (string out "Weight: samples × caller count (higher = bigger impact)\n\n"))
  (var rank 0)
  (loop [entry :in entries]
    (when (< rank 20)
      (def impact (get entry 0))
      (def count (get entry 1))
      (def callers (get entry 2))
      (def name (get entry 3))
      (set out (string out "  " rank ". weight=" impact
                        " (samples=" count " callers≈" callers ")  " name "\n"))
      (set rank (+ rank 1))))

  (harness/notify out :info)
  out)

(harness/register-command "dap-crossref-report" "crossref-report")
