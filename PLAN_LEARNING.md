# Learning Loop — Per-Project Memory, Skills, and Self-Improvement

Port Hermes-agent's learning architecture to dirge, adapted for the coding
context. Hermes stores memory globally in `~/.hermes/`; dirge stores it
**per-project** in `.dirge/` at the project root.

**CRITICAL RULE**: This is a PORT, not a redesign. When the plan says
"implement X," it means "read the Hermes source for X, understand it, and
write equivalent Rust that preserves every guard clause, edge case, and
error path." Do NOT invent a "simpler version" or "improve" on Hermes's
design. The bugs Hermes fixed in production are baked into its edge cases
— skipping them reintroduces those bugs.

## Architecture Overview

```
┌─────────────────────────────────────────────────┐
│ Layer 4: Curator (periodic skill maintenance)    │
│   agent/curator.py — lifecycle, consolidation    │
├─────────────────────────────────────────────────┤
│ Layer 3: Skill System (procedural memory)        │
│   tools/skill_manager_tool.py — CRUD + patches   │
│   tools/skill_usage.py — telemetry + provenance  │
├─────────────────────────────────────────────────┤
│ Layer 2: Memory Store (declarative memory)       │
│   tools/memory_tool.py — MEMORY.md, USER.md      │
├─────────────────────────────────────────────────┤
│ Layer 1: Background Review (the learning nudge)  │
│   agent/background_review.py — fork + evaluate   │
├─────────────────────────────────────────────────┤
│ Foundation: Session DB + Search + Compression    │
│   hermes_state.py — SQLite + FTS5                │
│   tools/session_search_tool.py — find past work  │
│   agent/context_compressor.py — long sessions    │
└─────────────────────────────────────────────────┘
```

---

## Current State Audit (What Exists vs What's Missing)

Before implementing anything, here is an honest assessment of every
subsystem. Each existing file was read and compared line-by-line against
its Hermes reference.

### Phase 0 — `.dirge/` Infrastructure ✅ COMPLETE

`src/extras/dirge_paths.rs` matches the spec: git-root walking,
`DIRGE_PROJECT_ROOT` env override, worktree detection, lazy directory
creation. No gaps.

### Phase 1 — Session DB: EXISTS BUT CRITICALLY INCOMPLETE

`src/extras/session_db.rs` has schema, WAL fallback, FTS5, migrations.
But these gaps make the session search tool almost useless:

- **GAP 1-A (CRITICAL)**: Messages are only written at session END, not
  per-turn. `ui/mod.rs:2941-2947` inserts exactly TWO messages (last user
  + last assistant) once at Done/AgentEnd. Hermes inserts every message
  at every turn. Without per-turn writes, session search returns almost
  nothing for the current session.
  *Fix*: call `db.insert_message()` after every turn in the UI event loop.

- **GAP 1-B**: FTS5 trigger only indexes `new.content`. Hermes indexes
  `COALESCE(content, '') || ' ' || COALESCE(tool_name, '') || ' ' ||
  COALESCE(tool_calls, '')`. Without tool_name/tool_calls in the index,
  searching for "bash" or "write" won't find tool-call messages.
  *Fix*: update `CREATE TRIGGER messages_ai` in `run_migration_v1()` (and
  add a v2 migration for existing DBs).

- **GAP 1-C**: No trigram FTS5 table for CJK/substring search.
  *Fix*: add `messages_fts_trigram` virtual table and backfill trigger,
  matching Hermes's `FTS_TRIGRAM_SQL`.

- **GAP 1-D**: No `_last_init_error` thread-safe error tracking.
  *Fix*: add `static LAST_INIT_ERROR: Mutex<Option<String>>`.

- **GAP 1-E**: Schema missing many Hermes fields. Sessions table lacks:
  `user_id`, `model_config`, `system_prompt`, `ended_at`, `end_reason`,
  `tool_call_count`, `cache_read_tokens`, `cache_write_tokens`,
  `reasoning_tokens`, `estimated_cost_usd`, `actual_cost_usd`,
  `cost_status`, `api_call_count`. Messages table lacks: `token_count`,
  `finish_reason`, `reasoning`, `reasoning_content`, `observed`.
  *Fix*: add migrations v2-v4 to bring schema to parity.

### Phase 2 — Memory Store: MOSTLY COMPLETE

`src/extras/memory_store.rs` has § delimiter, frozen snapshot, char
limits, substring matching, atomic writes, file locking, injection
scanning, drift detection, deduplication. Well-implemented.

- **GAP 2-A**: Threat patterns use simple `.contains()` substring checks.
  Hermes uses regex patterns (`re.compile`) for nuanced matching like
  `ignore\s+(previous|all|above|prior)\s+instructions`. This means things
  like "ignore  previous  instructions" (variable whitespace) are NOT
  caught.
  *Fix*: port Hermes's regex patterns exactly.

- **GAP 2-B**: Invisible unicode check is missing several bidi markers.
  Hermes lists: `\u200b`, `\u200c`, `\u200d`, `\u2060`, `\ufeff`,
  `\u202a`, `\u202b`, `\u202c`, `\u202d`, `\u202e`. Dirge lists only:
  `\u200b`, `\u200c`, `\u200d`, `\u2060`, `\u202e`, `\u202d`, `\u202c`.
  Missing: `\ufeff` (BOM/zero-width no-break space), `\u202a` (LRE),
  `\u202b` (RLE).
  *Fix*: add the missing characters from Hermes's `_INVISIBLE_CHARS`.

- **GAP 2-C**: `MemoryToolStore` wraps two `MemoryStore` instances but
  forces both to the same pitfall char limit. Hermes allows independent
  limits per target. Not critical but worth noting.

### Phase 3 — Skill System: SHELL EXISTS, GUTS MISSING

`src/extras/skills/` has manager.rs (CRUD), format.rs (YAML frontmatter),
guard.rs (security scanning). The `skill` tool is registered. But:

- **GAP 3-A (HIGH)**: No `.usage.json` sidecar (telemetry). Hermes's
  `tools/skill_usage.py` (608 lines) tracks `created_by`, `use_count`,
  `view_count`, `patch_count`, `last_*_at`, `state`, `pinned`,
  `archived_at`. This is the DATA the curator needs to make decisions.
  Without it, the curator is blind.
  *Fix*: create `src/extras/skills/usage.rs` — port `skill_usage.py` exactly.

- **GAP 3-B (HIGH)**: No fuzzy matching for patches. Hermes's
  `tools/fuzzy_match.py` handles whitespace normalization, indentation
  differences, escape sequences, block-anchor matching. Dirge's
  `manager.rs` does exact string find-and-replace. LLM-generated
  old_text almost never matches exactly.
  *Fix*: create `src/extras/skills/fuzzy_match.rs` — port `fuzzy_match.py` exactly.

- **GAP 3-C**: No provenance system. Hermes's `skill_provenance.py`
  distinguishes bundled (shipped) from agent-created skills. Dirge has no
  such distinction — every skill is treated equally, meaning bundled
  skills could be deleted or auto-archived by the curator.
  *Fix*: create `src/extras/skills/provenance.rs` — port `skill_provenance.py`.

- **GAP 3-D**: No archive/restore. Hermes moves archived skills to
  `.archive/`, state set to "archived", recoverable. Dirge's `delete`
  removes the directory completely.
  *Fix*: add `archive` and `restore` methods to `SkillManager`.

- **GAP 3-E**: No pinned skill support.
  *Fix*: add `pin`/`unpin` to usage tracking.

- **GAP 3-F**: `src/extras/skills/format.rs` reimplements YAML
  frontmatter parsing from scratch with string splitting. Hermes uses
  Python's `yaml` library. This is fragile — it won't handle all valid
  YAML frontmatter forms. Use `serde_yaml` instead.

### Phase 4 — Background Review: EXISTS BUT SIMPLIFIED

`src/agent/review.rs` has fork pattern, tool-limited runner,
fire-and-forget tokio task. But:

- **GAP 4-A**: Review prompt is 15 lines. Hermes's `_COMBINED_REVIEW_PROMPT`
  is ~90 lines with: class-level skill guidance, preference ordering
  (1. update loaded skill, 2. update umbrella, 3. add support file,
  4. create umbrella), signals that warrant action (corrections,
  frustration), what NOT to capture (env-dependent failures, negative
  claims, transient errors, one-off narratives). The current prompt is
  too terse to produce quality learnings.
  *Fix*: port Hermes's `_SKILL_REVIEW_PROMPT` and `_COMBINED_REVIEW_PROMPT`
  exactly, adapted for coding context.

- **GAP 4-B**: No action summary surfaced to user. Hermes scans the
  review agent's messages for successful tool actions and prints
  "💾 Self-improvement review: created skill X · added memory entry Y."
  Dirge silently logs success/failure.
  *Fix*: after the review fork completes, scan its messages for tool
  actions and emit a summary.

- **GAP 4-C**: No per-agent prompt override. Hermes allows
  `agent._COMBINED_REVIEW_PROMPT` to override.
  *Fix*: check for a config field before using the default prompt.

- **GAP 4-D**: No auxiliary failure emission. Hermes emits
  `agent._emit_auxiliary_failure("background review", e)` so the UI can
  surface review failures.
  *Fix*: emit a warning event or log message the UI can pick up.

- **GAP 4-E**: No tool-action deduplication vs parent session. Hermes
  skips tool messages already present in `messages_snapshot` to avoid
  re-surfacing stale "created" messages from prior turns.
  *Fix*: filter review results against the original session transcript.

### Phase 5 — Session Search: EXISTS BUT INCOMPLETE

`src/extras/session_search.rs` has three-shape design (discover, scroll,
browse). But:

- **GAP 5-A**: No lineage deduplication. Hermes's `_resolve_to_parent()`
  walks `parent_session_id` chain to root. Dirge doesn't deduplicate at
  all — each split session appears separately.
  *Fix*: add `resolve_lineage_root()` and group results by root.

- **GAP 5-B**: No lineage rebinding in scroll. If the caller passes a
  parent `session_id` but the message lives in a child, Hermes silently
  rebinds. Missing.
  *Fix*: add the rebinding check in the scroll path.

- **GAP 5-C**: No FTS5 query sanitization. Hermes has
  `_sanitize_fts5_query()` that strips unmatched special characters,
  quotes hyphenated/dotted terms. Dirge uses raw user query — FTS5 syntax
  errors will crash the search.
  *Fix*: port `_sanitize_fts5_query()` exactly.

- **GAP 5-D**: No source exclusion. Hermes excludes `review-fork` from
  browse and search.
  *Fix*: add `AND source != 'review-fork'` to browse/search queries.

- **GAP 5-E**: No current session exclusion. Hermes excludes the active
  session's lineage.
  *Fix*: accept an optional `exclude_session_id` parameter and filter.

- **GAP 5-F**: No CJK detection. Hermes detects CJK characters and
  switches to the trigram FTS5 table. Missing (depends on GAP 1-C).
  *Fix*: after adding trigram table, add CJK detection.

- **GAP 5-G**: Session search is **effectively useless** until GAP 1-A
  is fixed. Messages aren't written per-turn, so there's almost nothing
  to search.

### Phase 6 — Curator: STUB EXISTS, DOES NOTHING

`src/extras/skills/curator.rs` has `Curator` struct, `should_run_now()`,
`apply_automatic_transitions()`, interval gating, first-run deferral.
But `apply_automatic_transitions()` is a stub — it only marks `last_run`,
it doesn't walk skills or transition them.

- **GAP 6-A (CRITICAL)**: `apply_automatic_transitions()` does not
  transition skills. It needs to: walk every agent-created skill, check
  activity timestamps from `.usage.json`, apply lifecycle rules (30d
  stale, 90d archive, reactivate on recent activity, skip pinned).
  *Fix*: port Hermes's `apply_automatic_transitions()` (curator.py lines
  252-296) exactly.

- **GAP 6-B**: No consolidation review fork. Hermes spawns a forked
  AIAgent for consolidation (overlapping skills → umbrella, patch
  outdated, add cross-references). Missing entirely.
  *Fix*: port `maybe_run_curator()` consolidation fork (curator.py lines
  299-1224), reusing the spawn_review_runner pattern from Phase 4.

- **GAP 6-C**: No pinned skill bypass. Because pinning doesn't exist
  (GAP 3-E), the curator can't skip pinned skills.

- **GAP 6-D**: No provenance filter. Without usage tracking (GAP 3-A),
  the curator can't distinguish bundled vs agent-created.

### Phase 7 — Context Compression: DECISION ENGINE EXISTS, COMPRESSION DOESN'T

`src/agent/agent_loop/context_manager.rs` has threshold constants,
decision engine (`decide_after_usage`, `estimate_turn_start`), multi-tier
fold logic. The `run.rs` loop logs fold recommendations. But:

- **GAP 7-A (CRITICAL)**: The fold flag is set but never acted upon.
  `run.rs:424-438` logs "context-manager: fold recommended" and
  "context-manager: forcing summary and ending turn" but never actually
  compresses anything. There's no `compress_context()` call, no auxiliary
  model, no summary generation.
  *Fix*: implement the actual compression: protect head+tail, build
  structured summary prompt, call auxiliary model, insert summary as
  system message, rotate session_id.

- **GAP 7-B**: No structured summary. Hermes produces "Resolved questions
  / Pending questions / Active task / Key decisions / Remaining work."
  *Fix*: port Hermes's summary template from context_compressor.py.

- **GAP 7-C**: No filter-safe preamble. Hermes prefixes summaries with
  a directive that this is REFERENCE, not active instructions.
  *Fix*: port `SUMMARY_PREFIX` from context_compressor.py lines 37-51.

- **GAP 7-D**: No tool output pruning before summarization. Hermes prunes
  old/large tool outputs to placeholders.
  *Fix*: add prune pass before the compression LLM call.

- **GAP 7-E**: No session splitting after compression. The schema has
  `parent_session_id` but nothing sets it during compression.
  *Fix*: after compression, create new session_id, set parent_session_id.

- **GAP 7-F**: The ContextOverflow recovery path (detect error → `/compress`
  → respawn) is better than nothing but not Hermes-equivalent. Hermes
  compresses IN the same run without breaking the user's flow.

### Phase 8 — Integration: PARTIALLY WIRED

- ✅ Memory loaded at session start (`builder.rs:145-155`)
- ✅ Background review at session end (`ui/mod.rs:2951-2955`)
- ✅ Curator check at session end (`ui/mod.rs:2958-2967`) — though it
  does nothing due to GAP 6-A
- **GAP 8-A**: Skills are NOT loaded into system prompt. `builder.rs:156`
  creates a `SkillManager` but never reads skill content into the
  preamble.
  *Fix*: after memory injection, list skills from `.dirge/skills/`, read
  SKILL.md for each, inject relevant ones into the system prompt.

- **GAP 8-B**: Session DB is only written at session end, not per-turn.
  Same as GAP 1-A.

- **GAP 8-C**: No skill usage counter bumps during the session.
  *Fix*: when the agent loads or patches a skill, bump its usage counters.

- **GAP 8-D**: No graceful degradation for missing `.dirge/`. If the
  directory can't be created, the session should continue with a warning,
  not fail.

---

## Implementation Plan

### Pre-work: Write Integration Tests First

Before touching any implementation, write integration tests that
exercise the full pipeline end-to-end. These tests define success.

**File**: `src/tests/learning_loop_tests.rs` (new)

```rust
// Test 1: Full pipeline — session creates messages, search finds them
#[tokio::test]
async fn full_pipeline_session_search_finds_messages() { ... }

// Test 2: Memory store — add, replace, remove, frozen snapshot
#[tokio::test]
async fn memory_store_crud_and_snapshot() { ... }

// Test 3: Skill CRUD — create, patch with fuzzy match, archive, restore
#[tokio::test]
async fn skill_crud_fuzzy_patch_archive_restore() { ... }

// Test 4: Background review — fork creates memory entry from transcript
#[tokio::test]
async fn background_review_creates_memory_from_session() { ... }

// Test 5: Curator — automatic transitions (stale → archive)
#[tokio::test]
async fn curator_transitions_stale_skills_to_archive() { ... }

// Test 6: Context compression — structured summary, session split
#[tokio::test]
async fn compression_produces_structured_summary_and_splits_session() { ... }
```

Each test MUST pass before the corresponding phase is marked complete.

---

### Round 1: Fix Critical Data Flow (GAPs 1-A, 1-B)

**This is the most impactful change.** Without per-turn message
persistence and proper FTS5 indexing, session search is useless and
background review has no transcript to search.

**Step 1: Write messages per-turn, not just at session end.**

File: `src/ui/mod.rs`

Find every place where a user message is submitted and an assistant
response completes. After each assistant turn (including tool calls),
call `db.insert_message()`. The session DB is already opened at session
end (`ui/mod.rs:2931`); open it at session start instead and hold the
connection (or open/close per-write — rusqlite `Connection` is Send but
not Sync, so a `Mutex<Connection>` or per-write open pattern).

Do NOT change `insert_session`/`insert_message` signatures — they work
correctly. Just call them more often.

**Step 2: Update FTS5 trigger to index tool_name + tool_calls.**

File: `src/extras/session_db.rs`

In `run_migration_v1()`, update the `CREATE TRIGGER messages_ai` to
match Hermes's `FTS_SQL`:

```sql
CREATE TRIGGER messages_ai AFTER INSERT ON messages BEGIN
    INSERT INTO messages_fts(rowid, content) VALUES (
        new.id,
        COALESCE(new.content, '') || ' ' ||
        COALESCE(new.tool_name, '') || ' ' ||
        COALESCE(new.tool_calls, '')
    );
END;
```

Same for `messages_au` (update trigger). Add a v2 migration that drops
and recreates the triggers for existing DBs, then backfills:

```sql
INSERT INTO messages_fts(rowid, content)
SELECT id, COALESCE(content, '') || ' ' ||
           COALESCE(tool_name, '') || ' ' ||
           COALESCE(tool_calls, '')
FROM messages;
```

**Tests**: After these changes, insert a message with `tool_name =
"bash"`, search for "bash", verify it's found. Insert messages across
turns, search for mid-session content, verify it's findable.

---

### Round 2: Complete the Session DB Schema (GAPs 1-C, 1-D, 1-E)

**Step 1: Add trigram FTS5 table for CJK/substring search.**

Port Hermes's `FTS_TRIGRAM_SQL` from `hermes_state.py:285-312`. The
trigram tokenizer (`trigram`) is built into SQLite — no extension needed.
Add a v3 migration that creates the table and backfills it.

**Step 2: Add `_last_init_error` thread-safe error tracking.**

File: `src/extras/session_db.rs`

```rust
static LAST_INIT_ERROR: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

pub fn last_init_error() -> Option<String> {
    LAST_INIT_ERROR.lock().unwrap().clone()
}
```

In `SessionDb::open()`, on error, store the message. In `ui/mod.rs` and
slash-command handlers, call `last_init_error()` for actionable messages.

**Step 3: Add missing schema fields via migrations.**

Add migration v4 that ALTER TABLE sessions ADD COLUMN for:
`ended_at`, `end_reason`, `tool_call_count`, `api_call_count`
(starter set — the rest can be added as needed).

Add migration v5 that ALTER TABLE messages ADD COLUMN for:
`token_count`, `finish_reason`.

Update `insert_session` and `insert_message` to accept these fields
(use `Option<T>` for optional ones, or add builder methods).

**Tests**: Open DB, run insert, verify new columns are writable and
readable. Test migration from v1 schema (create old DB, open, verify it
migrates cleanly).

---

### Round 3: Memory Store Threat Patterns (GAPs 2-A, 2-B)

**Step 1: Port Hermes's regex patterns exactly.**

File: `src/extras/memory_store.rs`

Replace the substring `THREAT_PATTERNS` with compiled regex patterns.
Hermes's `_MEMORY_THREAT_PATTERNS` (memory_tool.py:68-84) uses
`re.compile`. Port each pattern to Rust's `regex::Regex`:

```rust
static THREAT_PATTERNS: LazyLock<Vec<(Regex, &str)>> = LazyLock::new(|| {
    vec![
        (Regex::new(r"ignore\s+(previous|all|above|prior)\s+instructions").unwrap(), "prompt_injection"),
        (Regex::new(r"you\s+are\s+now\s+").unwrap(), "role_hijack"),
        // ... port every pattern from Hermes
    ]
});
```

In `validate_content()`, iterate patterns and check `re.is_match(&content)`.

**Step 2: Add missing invisible characters.**

Add `\u{fef}` (BOM), `\u{202a}` (LRE), `\u{202b}` (RLE) to the
invisible-char set.

**Tests**: Verify regex patterns catch: "ignore   previous
instructions" (extra spaces), "IGNORE ALL INSTRUCTIONS" (case variation
if Hermes's patterns are case-insensitive — check whether Hermes uses
`re.IGNORECASE`). Verify invisible chars are caught.

---

### Round 4: Skill System — Usage Tracking (GAP 3-A)

**Step 1: Create `src/extras/skills/usage.rs`.**

Port `tools/skill_usage.py` faithfully. Key structures:

```rust
pub struct SkillUsage {
    pub created_by: Option<String>,  // "agent" or null
    pub use_count: u64,
    pub view_count: u64,
    pub patch_count: u64,
    pub last_used_at: Option<String>,
    pub last_viewed_at: Option<String>,
    pub last_patched_at: Option<String>,
    pub created_at: String,
    pub state: SkillState,           // Active, Stale, Archived
    pub pinned: bool,
    pub archived_at: Option<String>,
}

pub struct UsageStore {
    path: PathBuf,        // .dirge/skills/.usage.json
    lock_path: PathBuf,   // .dirge/skills/.usage.json.lock
    data: HashMap<String, SkillUsage>,
}
```

Key methods (match Hermes's signatures exactly):
- `load(paths: &ProjectPaths) -> Result<Self>`
- `record_create(name: &str, created_by: &str)`
- `record_use(name: &str)` — bumps use_count, sets last_used_at
- `record_view(name: &str)` — bumps view_count
- `record_patch(name: &str)` — bumps patch_count
- `set_pinned(name: &str, pinned: bool)`
- `set_state(name: &str, state: SkillState)`
- `is_agent_created(name: &str) -> bool` — provenance filter
- `activity_age_seconds(name: &str) -> Option<u64>`
- `save() -> Result<()>` — atomic write via tempfile + rename

Use `fcntl` file locking (or `fs2` crate via `fs_atomic.rs`).
All counter bumps are best-effort: failures log at DEBUG and return
silently. A broken sidecar never breaks the tool call.

**Step 2: Wire usage tracking into the skill tool.**

File: `src/agent/tools/skill.rs`

After each `skill_view` call: `usage.record_view(name)`.
After each `skill_manage` create: `usage.record_create(name, "agent")`.
After each `skill_manage` patch: `usage.record_patch(name)`.
After each `skill_manage` edit: `usage.record_patch(name)`.

**Step 3: Wire usage tracking into skill load at session start.**

File: `src/agent/builder.rs`

When injecting skills into the system prompt (GAP 8-A), record a view
for each loaded skill.

**Tests**: Create skill → usage record appears with created_by="agent".
View skill → use_count increments. Patch skill → patch_count increments.
Agent-created skill → `is_agent_created()` returns true.
Null created_by → `is_agent_created()` returns false.
Concurrent writes → lock serializes.
File corruption → load recovers gracefully (starts fresh).

---

### Round 5: Skill System — Fuzzy Matching (GAP 3-B)

**Step 1: Create `src/extras/skills/fuzzy_match.rs`.**

Port `tools/fuzzy_match.py` exactly. The function signature:

```rust
pub fn fuzzy_find_and_replace(
    content: &str,
    old_text: &str,
    new_text: &str,
) -> Result<String, FuzzyMatchError>
```

The algorithm (from Hermes):

1. **Normalize whitespace**: replace all whitespace sequences (tabs,
   spaces, `\r`) with single spaces for comparison. Track original
   whitespace for the replacement.

2. **Try exact match first**: if `old_text` exists verbatim, do a simple
   string replace. This is the fast path.

3. **Normalize and try again**: normalize both `content` and `old_text`
   (collapse whitespace, strip trailing whitespace from each line) and
   try to find the normalized `old_text` in normalized `content`. Map
   the match position back to the original content and apply the
   replacement preserving original indentation.

4. **Line-anchored match**: split both into lines. For each line in
   `old_text`, find the best matching line in `content` (by normalized
   comparison). If all lines match in order, use the line spans to
   construct the replacement.

5. **Block-anchor match**: when `old_text` appears multiple times in the
   content (ambiguous), use surrounding context lines to disambiguate.
   The caller can pass additional lines above/below the target.

Return `FuzzyMatchError::NotFound` if no match, `FuzzyMatchError::Ambiguous`
if multiple matches with different content (include previews).

**Step 2: Wire fuzzy matching into `SkillManager::patch()`.**

File: `src/extras/skills/manager.rs`

Replace the current exact `str::replace` with `fuzzy_find_and_replace`.

**Tests**: Whitespace differences (tabs vs spaces) → match succeeds.
Indentation differences (2 spaces vs 4) → match succeeds.
Content with regex special chars → literal match, not regex.
Ambiguous match → error with previews.
No match → error with "no match found".

---

### Round 6: Background Review Prompt (GAP 4-A)

**Step 1: Port Hermes's full review prompts.**

File: `src/agent/review.rs`

Replace the current `COMBINED_REVIEW_PROMPT` with a faithful port of
Hermes's `_SKILL_REVIEW_PROMPT` (background_review.py:45-148) and
`_COMBINED_REVIEW_PROMPT` (background_review.py:150-158), adapted for
coding context:

- Replace "who the user is" with "what the project is"
- Replace "user preferences" with "project conventions"
- Keep the preference order (1. loaded skill, 2. umbrella, 3. support
  file, 4. new umbrella)
- Keep the "what NOT to capture" section (env-dependent failures,
  negative claims, transient errors, one-off narratives)
- Keep the "Signals that warrant action" section
- Keep the "Target shape of the library" section

This will make the prompt ~100 lines instead of ~15.

**Step 2: Emit action summary after review completes.**

After the review fork finishes, scan `review_runner.event_rx` for tool
calls (the fork emits `AgentEvent::ToolCall` events — collect them).
Format a one-line summary like Hermes's `_safe_print`:

```
💾 Self-improvement review: created skill project-build · added memory entry "cargo test --all-features" · patched skill project-conventions
```

Write this to the renderer so the user sees it.

**Step 3: Allow per-agent prompt override.**

Add a `review_prompt_override: Option<String>` field to the review
config. In `spawn_background_review`, check for override before using
the default.

**Tests**: Review fork with coding-specific transcript → creates a
memory entry about build commands. Review fork with a correction
("don't use async in this module") → patches the relevant skill.
Review fork failure → main session continues unaffected.
Action summary is non-empty when fork made changes.

---

### Round 7: Session Search Completion (GAPs 5-A through 5-F)

**Step 1: Add lineage resolution.**

File: `src/extras/session_db.rs`

```rust
pub fn resolve_lineage_root(&self, session_id: &str) -> Result<String, String> {
    // Walk parent_session_id chain to the root.
    // Port of Hermes's _resolve_to_parent() (hermes_state.py:2009-2014).
}
```

**Step 2: Add FTS5 query sanitization.**

File: `src/extras/session_search.rs`

```rust
fn sanitize_fts5_query(query: &str) -> String {
    // Port of Hermes's _sanitize_fts5_query() (hermes_state.py:2037-2083).
    // 1. Strip unmatched FTS5-special characters
    // 2. Wrap hyphenated and dotted terms in quotes
}
```

**Step 3: Add source exclusion and session exclusion.**

File: `src/extras/session_search.rs`

In `discover()`: add `AND s.source != 'review-fork'` to the search query.
In `discover()`: add `AND s.id NOT IN (lineage)` for the current session.
In `browse()`: add `AND source != 'review-fork'`.

**Step 4: Add lineage deduplication in discover.**

After getting FTS5 results, group by lineage root (call
`resolve_lineage_root()` for each result's session_id). Keep only the
highest-ranked result per lineage.

**Step 5: Add lineage rebinding in scroll.**

If `get_anchored_view(session_id, message_id)` returns "message not
found", check if any child session contains the message:

```rust
// Check children: sessions WHERE parent_session_id = session_id
// For each child, try get_anchored_view(child_id, message_id)
// If found, rebind and return
```

**Tests**: Search with query → results exclude review-fork sessions.
Search with compression chain → only one result per lineage.
Scroll with parent session_id + child message_id → rebinds to child.
FTS5 query with special chars → sanitized, doesn't crash.
Browse → recent sessions exclude review-fork.

---

### Round 8: Curator — Make It Actually Work (GAP 6-A through 6-D)

**Step 1: Implement `apply_automatic_transitions()`.**

File: `src/extras/skills/curator.rs`

Port Hermes's `apply_automatic_transitions()` (curator.py:252-296) exactly:

1. Load `.usage.json`
2. For each agent-created skill (`created_by == "agent"`):
   a. Compute `activity_age = now - max(last_used_at, last_patched_at)`
   b. If `activity_age > ARCHIVE_AFTER_STALE_DAYS` (90d): move to
      `.archive/`, set state=Archived, set archived_at
   c. Else if `activity_age > STALE_AFTER_DAYS` (30d): set state=Stale
   d. Else if state was Stale but `activity_age < STALE_AFTER_DAYS`:
      reactivate to Active
   e. If `pinned == true`: skip entirely
3. Save `.usage.json`
4. Update `.curator_state` (set `last_run`)

**Step 2: Implement archive/restore.**

File: `src/extras/skills/manager.rs`

```rust
pub fn archive(&self, name: &str) -> Result<(), String> {
    // Move skill directory from .dirge/skills/{name} to .dirge/skills/.archive/{name}
    // Update usage state to Archived
}

pub fn restore(&self, name: &str) -> Result<(), String> {
    // Move skill directory from .dirge/skills/.archive/{name} to .dirge/skills/{name}
    // Update usage state to Active
}
```

**Step 3: Implement `pin`/`unpin` in usage tracking.**

File: `src/extras/skills/usage.rs`

```rust
pub fn set_pinned(&mut self, name: &str, pinned: bool) -> Result<(), String>
```

**Tests**: Skill inactive for 90 days → archived. Skill inactive for 30
days → stale. Stale skill with recent activity → reactivated. Pinned
skill → unaffected. Bundled skill → unaffected. Archive → skill moves
to .archive/. Restore → skill moves back. Curator state saved after run.
Curator doesn't run before interval elapses.

---

### Round 9: Context Compression — Actually Compress (GAPs 7-A through 7-F)

**Step 1: Implement the compression function.**

File: `src/agent/compression.rs` (new)

```rust
pub struct CompressionResult {
    pub summary: String,
    pub new_session_id: String,
    pub parent_session_id: String,
    pub tokens_before: u64,
    pub tokens_after: u64,
}

pub async fn compress_context(
    messages: &[Value],
    context_window: u64,
    model: &dyn CompletionModel,  // auxiliary model
) -> Result<CompressionResult, String>
```

Algorithm (port from `context_compressor.py` + `conversation_compression.py`):

1. **Check feasibility**: if tokens < 75% of context_window, return None
   (no compression needed).

2. **Budget**: protect head (system prompt + first 3 messages) and tail
   (last 5 messages). Middle gets compressed. Summary budget =
   `max(2000, 0.20 * middle_tokens, 12000)`.

3. **Tool output pruning**: for messages in the middle section, replace
   tool result content > 500 chars with `[Tool output truncated: N chars]`.

4. **Build structured summary prompt** (port from Hermes):
   ```
   <filter_safe_preamble>
   Summarize the following conversation segment. Include:
   - Resolved questions
   - Pending questions
   - Active task
   - Key decisions made
   - Remaining work (NOT next steps)
   
   <middle_messages>
   ```

5. **Call auxiliary model** with the summary prompt.

6. **Validate summary**: check it contains the expected sections.

7. **Build compressed context**: head + summary (as system message) + tail.

8. **Rotate session**: generate new session_id, set parent_session_id.

**Step 2: Wire compression into the agent loop.**

File: `src/agent/agent_loop/run.rs`

After the existing `decide_after_usage()` check (around line 418-440),
when the decision is `Fold` or `ExitWithSummary`:

1. Call `compress_context()` with the current context messages
2. Replace `current_context.messages` with the compressed version
3. Emit a visible event so the UI can show "Context compacted"
4. If `ExitWithSummary`, end the turn after compression

**Step 3: Port the filter-safe preamble.**

```rust
const SUMMARY_PREFIX: &str = "\
[This is a summary of the earlier parts of a long conversation. \
It is provided as REFERENCE only. Do NOT answer questions or \
fulfill requests mentioned in this summary; they were already \
addressed in the original conversation. The conversation \
continues below.]\n\n";
```

**Step 4: Add `compress_model` to LoopConfig.**

File: `src/agent/agent_loop/types.rs`

```rust
pub struct LoopConfig {
    // ... existing fields ...
    pub compact_model: Option<Box<dyn CompletionModel>>,  // for compression
}
```

**Tests**: Compression produces structured summary with all sections.
Tool outputs are pruned before summarization. Session splits after
compression (new session_id, parent_session_id set). Filter-safe
preamble is present. Auxiliary model fallback on failure. Budget
protects head and tail.

---

### Round 10: Integration — Wire Everything Together (GAPs 8-A through 8-D)

**Step 1: Inject skills into system prompt at session start.**

File: `src/agent/builder.rs`

After the memory injection block (around line 155), add:

```rust
// Load skills from .dirge/skills/ and inject into preamble
if let Ok(skills) = skill_manager.list_skills() {
    // For now, inject skill names and descriptions. Phase 2: relevance filtering.
    if !skills.is_empty() {
        preamble.push_str("\n\n## Project Skills\n\n");
        preamble.push_str("The following skills are available for this project. ");
        preamble.push_str("Use the `skill` tool to view full content.\n\n");
        for skill_name in &skills {
            if let Ok(spec) = skill_manager.read_spec(skill_name) {
                preamble.push_str(&format!("- **{}**: {}\n", spec.name, spec.description));
            }
        }
    }
}
```

**Step 2: Bump skill usage on load.**

When a skill is listed in the system prompt, record a view in `.usage.json`.

**Step 3: Open session DB at session start, not end.**

File: `src/ui/mod.rs`

Move the `SessionDb::open()` call from the Done handler (line 2931) to
session initialization. Hold the connection in the UI state. Call
`insert_message()` after each turn.

**Step 4: Graceful degradation.**

- If `.dirge/` can't be created: log warning, continue without
  memory/skills/session DB
- If session DB open fails: log warning + `last_init_error()`, continue
  without persistence
- If memory files are corrupt: log warning, use empty store, allow writes
  to overwrite
- If background review fails: log, continue (already implemented)
- If compression fails: log warning, try with smaller budget, eventually
  truncate oldest messages

---

## Implementation Order

```
Round 1:  Fix session DB per-turn writes + FTS5 triggers    (GAPs 1-A, 1-B)
Round 2:  Complete session DB schema                         (GAPs 1-C, 1-D, 1-E)
Round 3:  Memory store threat patterns                       (GAPs 2-A, 2-B)
Round 4:  Skill usage tracking                               (GAP 3-A)
Round 5:  Skill fuzzy matching                               (GAP 3-B)
Round 6:  Background review prompt + action summary          (GAPs 4-A, 4-B)
Round 7:  Session search completion                          (GAPs 5-A through 5-F)
Round 8:  Curator — actually transition skills               (GAPs 6-A through 6-D)
Round 9:  Context compression — actually compress            (GAPs 7-A through 7-F)
Round 10: Integration — wire everything together             (GAPs 8-A through 8-D)
```

Rounds 3, 4+5 can be parallelized (different subsystems).
Round 7 depends on Round 1+2 (needs session DB messages + schema).
Round 8 depends on Round 4 (needs usage tracking).
Round 9 depends on the agent loop but not on Rounds 3-8.

---

## Porting Rules (MUST FOLLOW)

1. **Read the Hermes source first.** Before writing any Rust code for a
   function, open the corresponding Hermes file and read the entire
   function. Understand its edge cases. Then write the Rust equivalent.

2. **Port every guard clause.** If Hermes checks `if not old_text.strip()`
   before proceeding, the Rust code must check `if old_text.trim().is_empty()`.
   Every error message pattern from Hermes should be preserved.

3. **Match error messages.** When Hermes returns `"Multiple entries matched
   '{old_text}'. Be more specific."`, the Rust code should return the same
   string (or as close as idiomatic Rust allows).

4. **No "simplifications."** Do not skip the drift detection, the file
   locking, the atomic writes, or the injection scanning because "it's
   unlikely to happen." These exist because they DID happen in production.

5. **Tests must pass before marking a round complete.** The integration
   tests from the pre-work section are the acceptance criteria.

6. **Run `cargo test --bin dirge` after every round.** The full test
   suite must stay green. If a change breaks existing tests, fix them
   before moving on.

---

## Key Hermes Files Referenced

| File | Lines | Port to |
|------|-------|---------|
| `hermes_state.py` | 137 KB | `src/extras/session_db.rs` |
| `tools/memory_tool.py` | 690 | `src/extras/memory_store.rs` |
| `tools/skill_manager_tool.py` | 1034 | `src/extras/skills/manager.rs` |
| `tools/skill_usage.py` | 608 | `src/extras/skills/usage.rs` (NEW) |
| `tools/skill_provenance.py` | ~150 | `src/extras/skills/provenance.rs` (NEW) |
| `tools/fuzzy_match.py` | ~200 | `src/extras/skills/fuzzy_match.rs` (NEW) |
| `tools/skills_guard.py` | ~150 | `src/extras/skills/guard.rs` |
| `agent/background_review.py` | 593 | `src/agent/review.rs` |
| `agent/curator.py` | 1224 | `src/extras/skills/curator.rs` |
| `tools/session_search_tool.py` | 602 | `src/extras/session_search.rs` |
| `agent/context_compressor.py` | 1104 | `src/agent/compression.rs` (NEW) |
| `agent/conversation_compression.py` | 603 | `src/agent/compression.rs` (NEW) |
