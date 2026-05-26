---
description: When implementing multiple independent subsystems, use parallel background subagents to maximize throughput
triggers:
  - "multiple independent rounds"
  - "parallelize"
  - "spin up subagents"
  - "rounds that can be parallelized"
---

# Parallel Subagent Implementation

When a plan has multiple independent implementation rounds, launch them as background `task` subagents rather than implementing sequentially.

## When to use

- The plan explicitly says rounds can be parallelized (different subsystems, no shared state)
- Each round is self-contained: creates/modifies its own files, has its own tests
- Rounds don't depend on each other's output

## How to launch

For each independent round, fire a background task:

```
task background=true prompt="Implement Round N of PLAN.md — [description].

Read the plan at /path/to/PLAN.md lines X-Y.
Then read the Hermes reference at /path/to/hermes/file.py.

Modify /path/to/dirge/src/... with [detailed instructions].

Run `cargo test --bin dirge <filter>` and `cargo check --bin dirge`.

IMPORTANT: This is a PORT. Do not redesign — match Hermes exactly."
```

## Tracking

- Use `task_status` to poll completion (don't sleep-wait)
- Subagent completion arrives as `<system-reminder>` at next turn
- Implement serial dependencies (rounds that depend on others) synchronously while parallel tasks run
- Scan subagent output for test counts and any failures before declaring complete

## Pitfalls

- Subagents can't see each other's changes — ensure no file conflicts
- Imports added by one subagent may overlap with another's — reconcile after all complete
- If a subagent adds a module (`pub mod foo;`), the parent `mod.rs` must be updated — do this in the main thread after all subagents finish
