# Learnings: 001-bistec-architect-agent

Build learnings, spec gaps, and patterns discovered.

**Categories:** spec_gap | design_gap | pattern | best_practice | agent_issue

---

## [L1] design_gap — Live MiniCPM5-2B run: brief extraction took 47s, filled B...

**When:** 2026-09-23 01:49 UTC
**Category:** design_gap
**Priority:** medium
**Status:** pending

### Detail
Live MiniCPM5-2B run: brief extraction took 47s, filled BriefItem.sources with quotes in Mode A, and classified 'ship in 3 weeks' as timeline=normal (should be urgent)

### Action
Clear sources in Mode A post-parse; consider Jev context Choices (with not_stated) to double-check MiniCPM's context enums, as in Mode B small path

---

## [L2] design_gap — Store lacked unconfirm_brief/delete_decisions; pipeline w...

**When:** 2026-09-23 02:16 UTC
**Category:** design_gap
**Priority:** low
**Status:** pending

### Detail
Store lacked unconfirm_brief/delete_decisions; pipeline worked around with a needs_reconfirm flag and stage-result id filtering

### Action
Add explicit store operations in a follow-up if the workaround causes confusion

---

## [L3] design_gap — src-tauri/src/model/report.rs + model/mod.rs added by orc...

**When:** 2026-09-23 02:40 UTC
**Category:** design_gap
**Priority:** medium
**Status:** pending

### Detail
src-tauri/src/model/report.rs + model/mod.rs added by orchestrator (shared Report DTOs so T8/T9 could run in parallel) — not declared in any task

### Action
Declare shared files (Cargo.toml, model/mod.rs, vite config) in a wave-0 prep task next time

---

## [L4] design_gap — src-tauri/Cargo.toml edited outside T1 (deps prep, [[bin]...

**When:** 2026-09-23 02:40 UTC
**Category:** design_gap
**Priority:** medium
**Status:** pending

### Detail
src-tauri/Cargo.toml edited outside T1 (deps prep, [[bin]] entries, default-run) — shared file not declared by T2/T10/T14

### Action
Declare shared files (Cargo.toml, model/mod.rs, vite config) in a wave-0 prep task next time

---

## [L5] design_gap — vite.config.ts edited after T1 (vitest include scoped to ...

**When:** 2026-09-23 02:40 UTC
**Category:** design_gap
**Priority:** medium
**Status:** pending

### Detail
vite.config.ts edited after T1 (vitest include scoped to src/ because agent worktrees under .claude/ were picked up)

### Action
Declare shared files (Cargo.toml, model/mod.rs, vite config) in a wave-0 prep task next time

---

## [L6] design_gap — src/lib/contract.test.ts + tests/fixtures/contract/*: cro...

**When:** 2026-09-23 02:40 UTC
**Category:** design_gap
**Priority:** medium
**Status:** pending

### Detail
src/lib/contract.test.ts + tests/fixtures/contract/*: cross-language contract check added in T10, not in the plan

### Action
Declare shared files (Cargo.toml, model/mod.rs, vite config) in a wave-0 prep task next time

---

## [L7] pattern — Parallel agents in git worktrees + orchestrator-authored ...

**When:** 2026-09-23 02:40 UTC
**Category:** pattern
**Priority:** high
**Status:** pending

### Detail
Parallel agents in git worktrees + orchestrator-authored shared skeleton (deps, module stubs, DTOs, IPC contract) avoided merge conflicts across 14 parallel tasks; only one conflict (decision.rs) occurred

### Action
Reuse: write shared types/contract before fanning out

---

## [L8] pattern — Rust-generated JSON fixtures parsed by the UI's Zod schem...

**When:** 2026-09-23 02:40 UTC
**Category:** pattern
**Priority:** medium
**Status:** pending

### Detail
Rust-generated JSON fixtures parsed by the UI's Zod schemas catch IPC drift; found AppError.stage null vs optional mismatch

### Action
Keep the contract test in CI (Rust first, then pnpm test)

---
