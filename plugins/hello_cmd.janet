# Plugin-registered slash command example
#
# Demonstrates the Phase 2 API:
#   (harness/register-command "cmd-name" "handler-fn-name")
#
# The handler is a Janet function taking a single args string (everything
# the user typed after the command name) and returning either nil or a
# string. Strings are printed in the chat; nil is silent.

(def hooks [])

(defn hello-handler [args]
  (if (= (length args) 0)
    "hello, world!"
    (string "hello, " args "!")))

(defn now-handler [_args]
  (string "epoch: " (os/time)))

(harness/register-command "hello" "hello-handler")
(harness/register-command "now" "now-handler")
