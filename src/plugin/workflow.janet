# dirge workflow plugin
# Adds architect → implementor → review workflow

(var phase :idle)

(defn on-init [ctx]
  (harness/log (string "workflow loaded (model: " (ctx :model) ")"))
  (set phase :idle)
  nil)

(defn on-prompt [ctx]
  (let [prompt (ctx :prompt)]
    # Detect feature requests
    (def pattern (string "\\b("
                    "add a feature|implement feature|add support for|"
                    "build a|create a|add .* to"
                    ")\\b"))
    (if (peg/match (peg/compile pattern) prompt)
      (do
        (harness/log "workflow: detected feature request")
        (harness/set-phase :architect)
        # Request architect prompt — harness will send this to LLM
        (harness/request-prompt (string
          "ARCHITECT MODE — Plan this feature step by step.\n\n"
          "1. First consider code layout — where should this code live?\n"
          "2. Produce a high-level plan as a mermaidjs diagram\n"
          "3. Plan file structure (list files to create/modify)\n"
          "4. Create function stubs and type signatures\n"
          "5. Write PLAN.md with the full plan\n\n"
          "Feature: " prompt))
        "Starting architect phase...")
      # After architect completes, check if we should enter implementor
      (if (= phase :architect)
        (do
          (harness/set-phase :implementor)
          (harness/request-prompt (string
            "IMPLEMENTOR MODE — Follow TDD strictly.\n\n"
            "1. Write a failing test first\n"
            "2. Implement minimal code to pass\n"
            "3. Run tests, verify green\n"
            "4. Refactor if needed\n\n"
            "Use the plan from the architect phase to guide implementation."))
          "Starting implementor phase...")
        (if (= phase :implementor)
          (do
            (harness/set-phase :review)
            (harness/request-prompt (string
              "REVIEW MODE — Review all changes, find and fix bugs.\n\n"
              "1. Review ALL changes made\n"
              "2. Check for bugs, edge cases, error handling\n"
              "3. Run tests and fix any failures\n"
              "4. Ensure TDD pattern was followed\n\n"
              "After review, fix any issues found."))
            "Starting review phase...")
          nil)))))

(defn on-response [ctx]
  # After each response, advance the phase
  (case phase
    :architect (set phase :implementor)
    :implementor (set phase :review)
    :review (set phase :idle)
    nil))
