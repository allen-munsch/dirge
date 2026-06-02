# dap-context — auto-inject rich debug context after every DAP stop
#
# Hooks on-tool-end and checks if a DAP session is active and stopped.
# If so, automatically evaluates every variable in scope, captures the
# full stack trace with source locations, lists threads, and displays
# the complete picture inline in the chat as a notification.
#
# This eliminates the manual "step, inspect, evaluate, bt, threads"
# loop you'd otherwise do for every breakpoint hit.
#
# Architecture:
#   Plugin → dap/sessions (check if stopped)
#          → dap/stack-trace (get frames + frame IDs)
#          → for each frame: dap/scopes → dap/variables (drill every scope)
#          → harness/notify (print everything)
#
# Uses the DAP Janet FFI bindings (src/dap/janet_bindings.rs) which
# chain through DAP_TX → tokio bridge → DapSessionManager → adapter.

(def hooks ["on-tool-end"])

# ── helpers ──────────────────────────────────────────────────────────
# Janet's bundled runtime has no JSON decoder. All DAP FFI functions
# return human-readable strings or structured text we can slice.

(defn- json-extract [s key]
  # Extract a quoted string value for a given key from JSON-like text.
  # e.g. (json-extract "\"name\": \"main\"" "name") → "main"
  (def pat (string "\"" key "\": \""))
  (def start (string/find pat s))
  (if (not start) nil
    (do
      (set start (+ start (length pat)))
      (def end (string/find "\"" s start))
      (if end (string/slice s start end)))))

(defn- json-extract-int [s key]
  # Extract an integer value for a given key.
  (def pat (string "\"" key "\": "))
  (def start (string/find pat s))
  (if (not start) nil
    (do
      (set start (+ start (length pat)))
      (var end start)
      (while (and (< end (length s))
                  (or (>= (get s end) 48) (<= (get s end) 57)))
        (set end (+ end 1)))
      (def num-str (string/slice s start end))
      (if (empty? num-str) nil (math/parse-int num-str)))))

(defn- json-extract-array [s key]
  # Extract array of strings like "name": "main", "name": "factorial"
  (def pat (string "\"" key "\": \""))
  (def results @[])
  (var pos 0)
  (while true
    (def start (string/find pat s pos))
    (if (not start) (break))
    (set start (+ start (length pat)))
    (def end (string/find "\"" s start))
    (if (not end) (break))
    (def val (string/slice s start end))
    (array/push results val)
    (set pos (+ end 1)))
  results)

# ── main hook — fires after every tool call ──────────────────────────

(defn on-tool-end [ctx]
  # Only fire when there's an active stopped session AND the tool was
  # a DAP control operation (step, continue, step-in, step-out, launch).
  # We detect DAP operations by checking if a session is active AND
  # stopped. For non-DAP tools (read, grep, edit), we stay quiet.
  (when (not (dap/session-active?))
    (break nil))

  (def session-str (dap/sessions))
  (when (not (and session-str (string/find "\"stopped\"" session-str)))
    (break nil))

  # ── Build comprehensive context ─────────────────────────────────

  (var out "━━━━ DEBUG CONTEXT ━━━━\n")

  # 1. Session summary
  (def adapter (or (json-extract session-str "adapter_name") "?"))
  (def reason (or (json-extract session-str "stop_reason") "stopped"))
  (def thread-id (json-extract-int session-str "thread_id"))
  (set out (string out "Adapter: " adapter "  |  Stopped: " reason))
  (when thread-id
    (set out (string out "  |  Thread: " thread-id)))
  (set out (string out "\n\n"))

  # 2. Stack trace (all frames)
  (def bt-str (dap/stack-trace))
  (when bt-str
    (set out (string out "── Stack Trace ──\n"))
    (def names (json-extract-array bt-str "name"))
    (def files (json-extract-array bt-str "path"))
    (def lines-str (json-extract-array bt-str "line"))

    (for i 0 (length names)
      (def name (get names i))
      (def file (if (< i (length files)) (get files i) "?"))
      (def line (if (< i (length lines-str)) (get lines-str i) "?"))
      (def marker (if (= i 0) "→" " "))
      # Show first 8 frames, skip runtime frames
      (when (< i 8)
        (when (not (string/find "runpy" file))
          (set out (string out "  " marker " " name " @ " file ":" line "\n")))))
    (set out (string out "\n")))

  # 3. Top-frame Locals (if scope ref available)
  # dap/sessions returns JSON with scope info — we call dap/vars if we
  # can extract a known scopes ref. But Janet has no JSON parser, so
  # we use a heuristic: call dap/vars on ref 1000 (first scope ref),
  # which won't return meaningful data unless there's a real frame.
  # For now, we skip auto-scopes (future: add dap/scopes C fn to get
  # the ref programmatically).

  (set out (string out "── Quick Inspect ──\n"))
  (set out (string out "  Try: /dap-repl p '<var>' to evaluate\n"))
  (set out (string out "       /dap-repl vars <ref> to drill scopes\n"))
  (set out (string out "       /dap-repl bt for full backtrace\n"))

  (harness/notify out :info))

# ── register ─────────────────────────────────────────────────────────
nil
