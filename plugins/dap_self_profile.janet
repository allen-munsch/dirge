# dap_self_profile.janet — profile dirge itself via DAP attach
#
# Registers /dap-self-profile start|stop|report for sampling dirge's
# own call stack. Uses the DAP attach Janet FFI (dap/attach) to
# connect lldb-dap to dirge's own PID, then periodically samples
# the stack trace, aggregating by function name.
#
# Architecture:
#   /dap-self-profile start →
#     Find dirge's own PID (os/getpid is not available in Janet,
#     so we accept a --pid flag or prompt the user)
#     (dap/attach <pid> "lldb-dap") →
#     Register on-tool-end hook that samples (dap/stack-trace)
#     each time the debuggee stops →
#     After N samples, write profile to .dirge/profiles/dirge-<ts>.txt
#
# Token savings: zero — this runs entirely in the background.
# The profile is consumed by the next agent session or by a human
# reading the report file.

(def hooks ["on-tool-end"])

(var profiling false)
(var profile-samples 0)
(var profile-max 50)
(var profile-counts @{})

# ── hook — sample on tool-end if active ──────────────────────────────

(defn on-tool-end [ctx]
  (when (not profiling) (break nil))
  (when (not (dap/session-active?)) (break nil))

  (def bt-str (dap/stack-trace))
  (when bt-str
    # Manual extraction of function names from JSON-ish output
    # (same technique as dap_profiler.janet)
    (var pos 0)
    (var found 0)
    (while (and (< found 10) (>= pos 0) (< pos (length bt-str)))
      (def ns (string/find "\"name\": \"" bt-str pos))
      (if (not ns) (break))
      (set ns (+ ns 9))
      (def ne (string/find "\"" bt-str ns))
      (when ne
        (def name (string/slice bt-str ns ne))
        (when (and (not (string/find "___lldb" name))
                   (not (string/find "_start" name))
                   (not= name "??"))
          (def c (get profile-counts name 0))
          (put profile-counts name (+ c 1))
          (set found (+ found 1)))
        (set pos (+ ne 1))))

    (set profile-samples (+ profile-samples 1))
    (when (>= profile-samples profile-max)
      (set profiling false)
      (dap/terminate)
      (def report (generate-report))
      (harness/notify report :info)
      (write-report-file report))))

# ── report ───────────────────────────────────────────────────────────

(defn- generate-report []
  (def entries @[])
  (loop [[k v] :pairs profile-counts]
    (array/push entries [v k]))
  (sort entries (fn [a b] (> (get a 0) (get b 0))))

  (var out "DIRGE SELF-PROFILE REPORT\n")
  (set out (string out "Samples: " profile-samples " / " profile-max "\n\n"))
  (var rank 0)
  (loop [entry :in entries]
    (when (< rank 20)
      (def count (get entry 0))
      (def name (get entry 1))
      (def pct (math/round (* 100 (/ count profile-samples))))
      (set out (string out "  " rank ". " pct "%  " name " (" count " samples)\n"))
      (set rank (+ rank 1))))
  out)

(defn- write-report-file [report]
  (def dir ".dirge/profiles")
  (def file (string dir "/dirge-" (os/time) ".txt"))
  (harness/log (string "dap-self-profile: writing to " file))
  (try
    (do
      (def f (file/open file :w))
      (when f
        (file/write f report)
        (file/close f)))
    ([_] (harness/log "dap-self-profile: failed to write profile file"))))

# ── slash commands ──────────────────────────────────────────────────

(defn self-profile-cmd [args]
  (def sub (if (empty? args) "start" (string/split " " (get args 0) args)))

  (match sub
    "start" (do
      (when (not (dap/available?))
        (break "DAP not available — build with --features dap,plugin"))
      (def pid-str (if (>(length args) 1) (get args 1) nil))
      (when (not pid-str)
        (break "usage: /dap-self-profile start <pid>"))
      (def pid (math/parse-int pid-str))
      (when (not pid)
        (break (string "invalid pid: " pid-str)))

      (def result (dap/attach pid "lldb-dap"))
      (if result
        (do
          (set profiling true)
          (set profile-samples 0)
          (set profile-max 50)
          (set profile-counts @{})
          (string "Self-profile started — " profile-max " samples at pid " pid))
        "Failed to attach — check ptrace_scope and that lldb-dap is installed"))

    "stop" (do
      (when (not profiling)
        (break "Self-profiler not running"))
      (set profiling false)
      (dap/terminate)
      (generate-report))

    "report" (if profiling
      (generate-report)
      "Self-profiler not running — start with /dap-self-profile start <pid>")

    "clear" (do
      (set profile-counts @{})
      (set profile-samples 0)
      "Profile data cleared")

    (string "unknown: " sub " — try start, stop, report, clear")))

(harness/register-command "dap-self-profile" "self-profile-cmd")
