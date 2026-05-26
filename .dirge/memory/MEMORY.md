## AgentEvent variant addition checklist
When adding new variant to `src/event.rs`:
1. `bridge.rs` — `translate()` match + `agent_event_kind` test helper
2. `h7_smoke.rs` — `print_event()` match
3. `integration.rs` — `agent_event_kind()` match
4. `src/extras/acp/mod.rs` — ACP event loop match
5. `src/ui/mod.rs` — main handler + `#[cfg(feature = "loop")]` path
6. `src/provider/mod.rs` — `run_print` (wildcard ok)
7. `src/agent/review.rs` — (wildcard ok)
Compiler catches all non-exhaustive patterns. Run `cargo test --bin dirge` (1261 tests) after.
§
## Steering pipeline + dead code + DB patterns
- Steering: UI→interjection_queue→steering_from_queue→LoopMessage::User→MessageStart→bridge→AgentEvent::UserMessage→write_user_lines
- Dead code: NEVER `#![allow(dead_code)]` module-level. Delete legacy. `#[cfg(test)]` for test-only exports. `#[cfg_attr(not(feature))]` for feature gates. `#![allow(unused_imports)]` only in `agent_loop/mod.rs`
- FTS5 formula migration: DELETE FROM messages_fts + INSERT SELECT (NOT 'rebuild' — external content tables don't support it)
- Schema versioning: `PRAGMA user_version`, sequential migrate() checks
- Env var tests: `static ENV_LOCK: Mutex<()>` + `EnvGuard` RAII with Drop cleanup
§
## Learning loop implementation — status + porting rules
PLAN_LEARNING.md v2: 14 gaps, 10 rounds. Completed: R1 (FTS5 tool_name indexing, per-turn DB writes at all 4 boundaries), R2 (trigram FTS5, _last_init_error, v4/v5 schema columns, end_session()), R3 (regex threat patterns, +6 security tests), R4 (skill usage tracking sidecar, .usage.json, +11 tests), R6 (Hermes-complete review prompt, action summary, prompt override). Remaining: R5 (fuzzy matching — running), R7-10.
Porting rules: read Hermes first, port every guard clause, match error messages, no simplifications, test after every round. FTS5 backfill: DELETE + INSERT SELECT (NOT 'rebuild'). Env var tests: Mutex + EnvGuard RAII.
§
Build commands: `cargo check --bin dirge` for type checking, `cargo test --bin dirge` for full test suite (1286 tests, 2.3s). Tests run in parallel by default — env var mutations need Mutex serialization. `cargo test --bin dirge <filter>` for targeted test runs.
