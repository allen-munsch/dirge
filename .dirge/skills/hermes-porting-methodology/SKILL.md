---
description: When implementing a feature that ports from Hermes Python to Rust, follow this methodology
triggers:
  - "port from Hermes"
  - "PLAN_LEARNING.md"
  - "implement round"
  - "Hermes reference"
---

# Hermes-to-Rust Porting Methodology

Systematic approach for porting Hermes Python features to dirge Rust.

## Phase 1: Audit (1-2 hours)

1. Read the PLAN_LEARNING.md section for the feature
2. Read the Hermes reference file(s) in full — every line
3. Read the corresponding dirge source — compare line by line
4. Enumerate gaps with severity: CRITICAL (non-functional), HIGH (wrong), MEDIUM (incomplete), LOW (cosmetic)
5. Document each gap with: Hermes line numbers, what Hermes does, what dirge does, why it matters

## Phase 2: Architecture (30 min)

1. Decide file layout — new files vs extending existing ones
2. Define Rust structs matching Hermes class shapes exactly
3. Plan migrations if schema changes needed (user_version gating, backfill)
4. Decide feature gates: `#[cfg_attr(not(feature = "X"), allow(dead_code))]` for feature-dependent code

## Phase 3: Implementation (per round)

1. **Stub**: Create files, define structs, stub methods with `todo!()`
2. **Core logic**: Port Hermes line by line — every guard clause, every error message
3. **Integration**: Wire into existing call sites
4. **Tests**: Write integration tests that exercise the full pipeline end-to-end
5. **Verify**: `cargo test --bin dirge <filter>`, `cargo check --bin dirge`

## Critical rules

- NEVER "simplify" — if Hermes has a guard clause, it caught a real bug
- Match error messages exactly to aid debugging
- FTS5 formula changes need DELETE + INSERT SELECT backfill (NOT `'rebuild'`)
- Schema migrations: sequential version checks, `IF NOT EXISTS` for triggers, handle "duplicate column name"
- Every round: `cargo test --bin dirge` must stay green
