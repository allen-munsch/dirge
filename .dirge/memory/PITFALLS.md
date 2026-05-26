Session DB pitfalls: (1) FTS5 external content tables — `'rebuild'` re-indexes using OLD trigger formula; to change indexed content need DELETE + INSERT SELECT. (2) `cargo test` parallelism: env var mutations need `Mutex<()>` serialization. (3) `SkillState` enum: `PartialEq` + move means check equality BEFORE assigning. (4) `atomic_write_sync` returns `Result<(), Error>` not `Result<(), String>` — need `.map_err`. (5) Migration chain: user_version gating, `IF NOT EXISTS` for FTS triggers, handle "duplicate column name" in ALTER TABLE.
§
Learning loop gaps: R1 fixed — per-turn session DB writes + FTS5 tool_name/tool_calls indexing + v2 backfill migration. Remaining: compression (fold flag unused), skill usage tracking, fuzzy patches, curator stub, skills in preamble. 14-gap audit in PLAN_LEARNING.md.
§
## FTS5 formula migration: 'rebuild' doesn't work
External-content FTS5: `INSERT INTO fts(fts) VALUES('rebuild')` re-indexes using old trigger formula. To change indexed content (e.g. add tool_name to index), DELETE FROM fts then INSERT INTO fts SELECT id, new_formula FROM messages.
§
## #![allow(dead_code)] hides real dead code
Module-level suppression in agent_loop/mod.rs and lsp/mod.rs concealed ~50 genuinely unused items. Removing it revealed the true extent. Prefer targeted per-item annotations — even many are better than module-wide silence.
§
## env::set_var + parallel tests = flaky
`std::env::set_var` is global/unsafe/unsynchronized. Tests mutating same key race. Fix: static Mutex + RAII EnvGuard that clears on Drop (applied in dirge_paths.rs).
