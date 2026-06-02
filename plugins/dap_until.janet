# dap_until.janet — auto-continue until condition evaluates to true
#
# Registers /dap-until <expression> — sets a watch condition that
# auto-continues (without chat noise) at each breakpoint until the
# condition evaluates to something truthy. When it flips, the plugin
# breaks into the chat with the current state.
#
# Replaces the manual "continue, check, continue, check..." loop
# with one break-in. Token-optimal for hunting value changes.
#
# Architecture:
#   1. User runs /dap-until "counter.value > 15"
#   2. Plugin stores the condition
#   3. Each on-tool-end (after a DAP stop) evaluates the condition
#      via (dap/eval <condition>)
#   4. If truthy → print state + notify, disable
#   5. If false → silently call (dap/continue)

(def hooks ["on-tool-end"])

(var until-active false)
(var until-condition "")
(var until-silent-skips 0)      # how many hits we silently continued past

(defn on-tool-end [_ctx]
  (when (not until-active) (break nil))
  (when (not (dap/session-active?)) (break nil))
  (def s-str (dap/sessions))
  (when (not (and s-str (string/find "\"stopped\"" s-str))) (break nil))

  (def result (dap/eval until-condition))
  (def triggered (if result
    (not (or (string/find "false" result)
             (string/find "null" result)
             (string/find "undefined" result)))
    false))

  (if triggered
    (do
      (set until-active false)
      (def bt (dap/stack-trace))
      (var out (string "🎯 UNTIL TRIGGERED: " until-condition
                       "  (after " until-silent-skips " silent skips)\n\n"))
      (when bt (set out (string out "Stack: " bt "\n")))
      (set out (string out "Value: " (or result "?")))
      (harness/notify out :info)
      (set until-silent-skips 0))
    (do
      (set until-silent-skips (+ until-silent-skips 1))
      (dap/continue))))

(defn until-cmd [args]
  (if (empty? args)
    (if until-active
      (string "dap-until tracking \"" until-condition
              "\" — " until-silent-skips " silent skips so far")
      "No until-expression active. Usage: /dap-until <expression>")
    (do
      (when (not (dap/session-active?))
        (break "No active DAP session — launch or attach first"))
      (set until-active true)
      (set until-condition (string/join (string/split " " args) " "))
      (set until-silent-skips 0)
      (string "🎯 dap-until watching \"" until-condition
              "\" — I'll break in when it becomes true"))))

(harness/register-command "dap-until" "until-cmd")
