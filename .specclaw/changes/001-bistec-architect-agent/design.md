# Design: BISTEC Architect — Tauri desktop app (local MiniCPM + Jev decisions)

**Change:** 001-bistec-architect-agent
**Created:** 2026-09-23

## Technical Approach

A Tauri 2 app with a **Rust core that owns the whole pipeline** and a React UI that only renders and edits. The UI talks to the core through typed Tauri commands and receives progress through Tauri events.

Both external model calls sit behind traits:

- `DecisionClient` is implemented by Jev over HTTP.
- `LocalModel` is implemented by Ollama over HTTP.

Every pipeline stage can therefore be unit-tested with fakes, and all arithmetic, routing, rules, and rendering are pure functions.

The pipeline is a sequence of stages. The result of each stage is persisted, so Retry resumes from the failed stage and History replays the report without any network calls:

```
input ─► [brief] ─► (user confirms) ─► [gates] ─► [platforms] ─► [rules] ─► [applicability] ─► [decisions] ─► [compose+route] ─► report
          MiniCPM / Jev-context          Jev        Jev            code        Jev                Jev            code
```

## Architecture

```
jev-architect/
├─ package.json, pnpm-lock.yaml, vite.config.ts, tsconfig.json, index.html
├─ src/                          React UI (TS strict, Tailwind v4, shadcn/ui, Zod, TanStack Query)
│  ├─ main.tsx, App.tsx          router: Home | Brief | Run | Report | Settings
│  ├─ lib/ipc.ts                 typed wrappers over invoke() + event listeners; the only file that touches @tauri-apps/api
│  ├─ lib/schemas.ts             Zod mirrors of the Rust DTOs (Brief, DecisionResult, Review, Settings)
│  ├─ screens/{Home,Brief,Run,Report,Settings}.tsx
│  ├─ components/{DecisionCard,ProbBars,ConfidenceMeter,ScoreHeatmap,ReviewDialog,DataNotice,HealthPanel}.tsx
│  └─ components/ui/…            shadcn generated
├─ e2e/smoke.spec.ts             Playwright on `vite preview` with mocked IPC (window.__TAURI_INTERNALS__ shim)
├─ catalog/                      bundled into the binary via include_str!
│  ├─ decision-types.yaml        27 types: 25 matrix rows + cloud-platform + backend-platform
│  ├─ criteria.yaml              6 criteria, 5-level rubrics, default weights
│  ├─ rules.yaml                 variant groups (FR-9) — data, interpreted by rules.rs
│  └─ adr-template.md            bistec-architect ADR template + Review + Evidence sections
├─ golden/*.yaml                 ≥10 calibration cases
├─ .github/workflows/ci.yml      matrix macos-latest, windows-latest
└─ src-tauri/
   ├─ Cargo.toml, tauri.conf.json, capabilities/default.json
   ├─ src/main.rs, lib.rs        builder, plugin registration, state
   ├─ src/commands.rs            #[tauri::command] surface (see API Changes)
   ├─ src/model/{brief,catalog,decision,review,settings}.rs   serde DTOs
   ├─ src/catalog.rs             load + validate YAML; alias matching for hold options
   ├─ src/jev.rs                 DecisionClient trait + HttpJev (reqwest, retries, Question/Answer types)
   ├─ src/ollama.rs              LocalModel trait + OllamaModel (/api/chat, format=JSON schema, /api/tags for health)
   ├─ src/docs.rs                pdf (pdf-extract), docx (zip + quick-xml), md/txt → Vec<Section>
   ├─ src/brief.rs               Mode A extraction, Mode B small (Jev context Choices) / large (per-section + merge)
   ├─ src/questions.rs           builders for gate / platform / applicability / decision questions; batching
   ├─ src/rules.rs               variant selection from rules.yaml + auth app-type mapping
   ├─ src/scoring.rs             composite, agreement, margin, routing → Route + reasons (pure)
   ├─ src/pipeline.rs            stage orchestration, budget fitting, precedent, events, resume-from-stage
   ├─ src/render.rs              ADR markdown, report markdown, self-contained HTML (minijinja templates)
   ├─ src/store.rs               rusqlite + migrations; sessions/decisions/reviews/jev_calls/settings
   ├─ src/secrets.rs             SecretStore trait + KeyringStore (keyring crate) + MemoryStore (tests)
   ├─ src/bin/catalog_check.rs   FR-22
   ├─ src/bin/calibrate.rs       FR-17
   └─ tests/fixtures/{docs,jev,adr}/…
```

### Jev request shapes (per jev-1.13 jaggedness guidance: literal, criteria per option, no arithmetic)

- **Gates** (1 call): `state = evidence`, 3 Nouls with explicit `criteria.true/false`.
- **Platforms** (1 call): a Choice for `cloud_platform` (azure / hetzner / hybrid, with the skill's "When to Use" text as criteria), a Choice for `backend_platform` (dotnet / node / other, where other = a Java or Python justification such as an ML workload), and a Choice for `auth_app_type` (internal_enterprise / b2b_saas / b2c / m2m). The auth Choice is only asked if an auth row is a candidate, and it is included speculatively in this same call (fan-out).
- **Applicability** (1 call): one Noul per candidate type, keyed `applies__<type_id>`.
- **Decisions** (⌈Q/64⌉ calls, 4 at a time): per type, `choice__<type>` plus `score__<type>__<option>__<criterion>`. The state for decision calls is `{ brief, evidence_sections?, bistec_standard: <type's option descriptions + rings>, bistec_precedent? }`, fitted to the budget in this order of removal: precedent, then low-priority evidence sections, then brief requirements (constraints and NFRs are kept longest).

In Mode B small, the Brief `context` is filled by one extra call made before the review step: a Choice per Context Assessment dimension, each including `not_stated`.

### Routing (scoring.rs, pure)

```
composite(o) = Σ_c w_c · (score_c(o) / 4) / Σ_c w_c        # score ∈ [0,4] from 5-level rubric
reasons = []
if choice.confidence < τ            → low_confidence
if argmax(composite) ≠ choice.choice → disagreement
if top1 − top2 < m                  → close_margin
if ring(choice) == hold             → hold_option
if gates.injection ≥ 0.3            → possible_injection
route = reasons.empty ? Proposed : NeedsArchitect
```

`lock_in` is phrased so that a higher level means *less* lock-in, which keeps the formula uniform.

## File Changes Map

| File | Action | Description |
|------|--------|-------------|
| `package.json`, `vite.config.ts`, `tsconfig*.json`, `index.html`, `components.json` | create | Vite + React TS strict + Tailwind v4 + shadcn config; scripts `dev`, `build`, `test`, `e2e`, `tauri` |
| `src/**` | create | UI as listed in Architecture |
| `e2e/smoke.spec.ts`, `playwright.config.ts` | create | AC-18 |
| `src-tauri/**` | create | Rust core as listed in Architecture |
| `catalog/*.yaml`, `catalog/adr-template.md` | create | Converted from `bistec-architect` SKILL.md |
| `golden/*.yaml` | create | ≥10 calibration cases (AC-19) |
| `.github/workflows/ci.yml` | create | AC-1 |
| `.gitignore` | modify | add `node_modules`, `dist`, `src-tauri/target`, `*.sqlite` |
| `README.md` | create | prerequisites (Ollama + `ollama pull`), key setup, data notice, dev/build commands |

## Data Model Changes

SQLite (`app_data_dir/architect.sqlite`), migrations embedded (`rusqlite_migration`):

| Table | Columns |
|---|---|
| `settings` | `key TEXT PK, value TEXT` (JSON values; **never** the API key) |
| `sessions` | `id TEXT PK (uuid), created_at, title, mode ('describe'|'upload'), input_text, doc_name, sections_json, brief_json, brief_confirmed_at, stage ('brief'|'gates'|…|'done'|'failed:<stage>'), error` |
| `stage_results` | `session_id, stage, result_json, created_at` — PK (session_id, stage); drives resume + replay |
| `decisions` | `id TEXT PK, session_id, type_id, choice_json, scores_json, composite_json, route, reasons_json, model_snapshot, request_hash, adr_number NULL` |
| `reviews` | `id INTEGER PK, decision_id, action ('accept'|'override'|'reject'), option_id, reviewer, reason, at_utc` — append-only (no UPDATE/DELETE in code; a trigger rejects them) |
| `jev_calls` | `id INTEGER PK, session_id, stage, input_tokens, output_tokens, cost_usd NULL, model_snapshot, at_utc` |

Precedent query: the latest review per decision is `accept`/`override`, `type_id = ?`, ordered by the review's `at_utc` descending, limit 3.

## API Changes

Tauri commands (all `async`, errors as `{ code, message, stage? }`):

| Command | In → Out |
|---|---|
| `get_settings` / `save_settings` | `Settings` (no key field) |
| `set_api_key(key)` / `clear_api_key()` / `has_api_key() → bool` | FR-2 |
| `ack_data_notice()` / `data_notice_acked() → bool` | FR-3 |
| `health_check() → Health { ollama, model_present, pull_command, openrouter }` | FR-4 |
| `start_describe(text) → session_id` | Mode A: builds the Brief |
| `start_upload(path) → session_id` | Mode B: parses, routes small or large, builds the Brief |
| `get_session(id) → SessionView` | brief, sections, stage, report if done |
| `update_brief(id, brief)`, `confirm_brief(id)` | FR-8 |
| `run_decisions(id)` / `retry(id)` | runs or resumes the pipeline, emits `pipeline://progress { session_id, stage, done, total }` |
| `review(decision_id, action, option_id?, reason?) → Review` | FR-18 |
| `list_sessions() → [SessionSummary]` | FR-21 |
| `export_adrs(id, dir) → [path]`, `export_report(id, dir, format: md|html) → path` | FR-19/20 |

Capabilities: `dialog:allow-open`, `dialog:allow-save`, core events. Only `reqwest` in Rust talks to the network. The webview CSP is `default-src 'self'`, so the UI makes no network requests itself.

## Key Decisions

1. **Rust owns the pipeline, not the UI.** This keeps the key, the network, and the arithmetic in one trusted process, and makes NFR-3 and NFR-5 enforceable. The UI never sees the key and never calls external hosts.
2. **Decisions API over plain HTTP, with no SDK.** It is simple JSON, it returns `usage.cost` for exact cost reporting, and there is no official Rust SDK.
3. **Traits for both models.** All tests run offline and deterministically. Live calls happen only in `calibrate` and in manual QA.
4. **Rules as data (`rules.yaml`).** Variant-row selection mirrors the skill's structure and can be reviewed by architects without reading Rust. Rules only select rows and never pick a technology.
5. **Stage results are persisted.** This gives Retry-from-stage and exact History replay (AC-16) for free, and records exactly what Jev was asked and answered.
6. **Append-only reviews enforced by a DB trigger**, not only by code convention (AC-13).
7. **PDF via the print dialog, not a PDF library.** This avoids a heavy dependency. Markdown and self-contained HTML are the formats with guaranteed output.
8. **Tauri CLI v2 is pinned in devDependencies.** The global `cargo-tauri` on this machine is 1.5 and must not be used.

## Risks & Mitigations

| Risk | Mitigation |
|---|---|
| The 2B model invents or omits constraints | JSON-schema-constrained output, `unknown` allowed, and the mandatory review step (FR-8) |
| The Jev request format or field names differ from the tutorial | A single `jev.rs` module with fixture-based tests. A live smoke test runs in `health_check`, and `calibrate` runs before release |
| Question volume and cost (27 types × up to ~4 options × 6 criteria) | Applicability pruning, batching of 64 questions, and the actual cost shown per session. Calibrate reports cost per case |
| Thresholds and weights are uncalibrated | They are labelled "uncalibrated defaults" in Settings until `calibrate` has been run. Nothing is Proposed without a human reviewing it anyway |
| Prompt injection inside uploaded documents | The injection gate forces Needs architect, and human approval is mandatory |
| PDF text extraction varies | Fixture tests; a clear error when there's no text layer; DOCX/MD/TXT recommended in the UI |
| No Windows machine for local testing | CI `windows-latest` builds and runs the tests. A manual Windows QA pass is listed as a release follow-up |
| Keychain prompts on macOS in unsigned dev builds | Documented in the README. Signing and notarisation are out of scope |

## Grounding sources

- `bistec-architect/SKILL.md`: Technology Selection Matrix (→ `decision-types.yaml`), Cloud Platform and Backend Stack tables (→ the two platform types), Auth Decision Tree (→ `auth_app_type` + `rules.yaml`), Context Assessment (→ the Brief `context` enum values), ADR template (→ `adr-template.md`), and the line *"Every deviation must be documented with rationale in the Architecture Decision Record"* (→ trial/hold options always produce a Bistec Alignment note).
- TypeSafe `model-jaggedness/jev-1.13`: *"Keep the arithmetic in code"*, *"Filter first; send only what the question needs"*, *"Write the exact condition, criteria for each available options"* (→ scoring.rs, budget fitting, question builders).
- OpenRouter Jev tutorial: the request and response shape including `usage.cost`, and *"Pin `typesafe/jev-1.13` when you need thresholds tuned against one specific version to stay stable"* (→ the pinned default).
