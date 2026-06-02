# dap_profiler_crossref.janet — profiler extended with LSP call counts
#
# Extends the dap_profiler.janet statistical profiler with LSP reference
# lookups. After collecting the top-20 hotspot report, for each function
# we query (harness/lsp-references file line char) to count how many
# call sites exist across the project. Functions with many callers are
# higher-impact optimization targets.
#
# Uses the typed LSP wrapper `harness/lsp-references` which takes
# file, line, char. Guarded by `(harness/lsp?)` — silent no-op when
# LSP is unavailable.
#
# Architecture:
#   /dap-crossref-report → read dap_profiler's counts (same Janet runtime) →
#   for each hot function → (harness/lsp-references file line char) [guarded] →
#   count callers → sort by impact (samples × caller count) →
#   present weighted hotspot report

(def hooks [])

# ── reference the profiler's state from dap_profiler.janet ────────────
# Janet modules share globals — if dap_profiler.janet loaded before this
# plugin, profile-counts, profile-samples, and profile-interval are
# accessible here without imports.

(defn crossref-report [args]
  (when (not (dap/session-active?))
    (break "No active DAP session — data only available during a session"))

  (var counts (dyn :profile-counts))
  (var samples (dyn :profile-samples))
  (if (not counts)
    (set counts @{})
    (set samples 0))

  (when (empty? counts)
    (break "No profile data. Run /dap-profile first to collect samples."))

  # Guard: only call LSP if available. The C function expects exactly
  # 5 arguments (op, file, line, char, query) and segfaults on anything
  # else. With this guard, LSP-unavailable builds get a sample-count-only
  # report instead of a crash.
  (def lsp-ok (and (harness/lsp?)
                   (dyn :lsp-available)))  # per project memory guard

  (def entries @[])
  (loop [[k v] :pairs counts]
    (def name k)
    # Parse function name to extract file:line (if the key contains that info)
    # For now, use line=1 char=1 as defaults — the references lookup will
    # find all references to the symbol name in the file.
    (var refs-str nil)
    (when lsp-ok
      (set refs-str (harness/lsp-references name 1 1)))

    (var ref-count 1)  # default to 1 if LSP unavailable or no references
    (when (and refs-str (not (string/find "nil" refs-str)))
      # Count references by counting "name" occurrences in the JSON result
      (var count 0)
      (var pos 0)
      (while (>= pos 0)
        (def found (string/find "\"name\"" refs-str pos))
        (if found
          (do (set count (+ count 1)) (set pos (+ found 6)))
          (set pos -1)))
      (set ref-count (max 1 count)))

    (def impact (* v ref-count))
    (array/push entries [impact v ref-count name]))

  (sort entries (fn [a b] (> (get a 0) (get b 0))))

  (var out "CROSS-REFERENCED PROFILE REPORT\n")
  (if lsp-ok
    (set out (string out "Weight: samples × caller count (higher = bigger impact)\n\n"))
    (set out (string out "LSP unavailable — showing sample counts only\n\n")))
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
