# Spec: BISTEC Architect — Tauri desktop app (local MiniCPM + Jev decisions)

**Change:** 001-bistec-architect-agent
**Created:** 2026-09-23
**Status:** 🟡 Draft

## Overview

A desktop application for macOS and Windows. A BISTEC user either **describes a project** or **uploads a requirements document**. The app returns a **report of technology decisions aligned with BISTEC standards**. Each decision shows a probability for every option, a confidence value, and a route (Proposed / Needs architect). A named reviewer must approve each decision, and approved decisions export as ADRs in the `bistec-architect` template.

Three roles, each held by one component:

- **MiniCPM5-2B** (local, via Ollama) *understands*: it extracts a structured brief from free text or from large documents.
- **Jev** (`typesafe/jev-1.13`, via the OpenRouter Decisions API) *decides*: typed Choice, Score, and Noul answers against the BISTEC catalogue.
- **Rust code** *controls*: rules, arithmetic, routing, persistence, rendering. Jev never does arithmetic and MiniCPM never makes decisions.

### Stated assumptions

- **A1:** The "Quick Reference: Technology Selection Matrix" in `bistec-architect` has **25** requirement rows, not 26 as the proposal said (the proposal miscounted). v1 covers all 25, plus two decision types drawn from the skill's own tables: **cloud-platform** (Cloud Platform table) and **backend-platform** (Backend Stack table). The skill's rows depend on these two; see FR-9.
- **A2:** Matrix cells that list two technologies (e.g. "Zustand / TanStack Query", "SignalR (.NET) / Socket.IO (Node)") become **separate options with the same ring**. A cell of "—" contributes no option.
- **A3:** Jev is called through `POST https://openrouter.ai/api/alpha/decisions`. The response carries `answers`, `model` (a dated snapshot), and `usage.{input_tokens, output_tokens, cost}`, as shown in the OpenRouter Jev tutorial.
- **A4:** Tokens are estimated as `ceil(chars / 4)`. The default state budget is 20,000 tokens, leaving headroom for the question payload within Jev's 32k context. The budget is configurable.
- **A5:** The local model is `hf.co/openbmb/MiniCPM5-2B-GGUF:Q4_K_M`, served by Ollama at `http://127.0.0.1:11434`. Both values are configurable.
- **A6:** Starting weights and thresholds are defaults we draft from `bistec-architect` preferences. They are **uncalibrated** until the golden-set run in FR-17 has been done against the live API.

## Requirements

### Functional Requirements

**Setup and settings**
- **FR-1 Settings:** the user can set and persist:
  - OpenRouter API key
  - Jev model (default `typesafe/jev-1.13`)
  - Ollama base URL and model name
  - state token budget
  - confidence threshold (default 0.5) and minimum composite margin (default 0.05)
  - weight per criterion
  - reviewer display name
- **FR-2 Key storage:** the API key is stored in the OS credential store (macOS Keychain / Windows Credential Manager). The UI can only set it, clear it, or ask whether one is set. It can never read the key back.
- **FR-3 First-run data notice:** before the first Jev call, the user must acknowledge a notice that brief and document content is sent to OpenRouter and TypeSafe **without redaction**. The acknowledgement is persisted. Settings shows the same notice.
- **FR-4 Health check:** a "Check connections" action reports three things separately: Ollama reachable, local model present, and OpenRouter key valid (a minimal one-Noul Jev call). When the model is missing it shows the exact `ollama pull …` command.

**Input**
- **FR-5 Mode A (describe):** free text → MiniCPM, through Ollama `/api/chat` with a JSON-schema `format` → a **Brief**:
  - `context`: scale, budget, timeline, team_size, data_sensitivity, each one of the skill's Context Assessment values or `unknown`; compliance is a list
  - `requirements[]`, `nfrs[]`, `constraints[]`, `team_skills[]`, `mentioned_technologies[]`

  Output that is malformed or fails schema validation is retried once, then reported as an error. It is never partially accepted.
- **FR-6 Mode B (upload):** accepts `.pdf` (text layer), `.docx`, `.md`, and `.txt`. The document is converted to text and split into **sections** with stable ids (`S1…Sn`), using headings where they exist and ~1,500-token chunks otherwise.
- **FR-7 Mode B size routing:**
  - **Small** (estimated tokens ≤ budget): the sections themselves are the evidence. The Brief's `context` dimensions are filled by **Jev Choice** questions, each with a `not_stated` option.
  - **Large** (> budget): **MiniCPM extracts a partial Brief per section**, with every item tagged with its section id. The partial Briefs are merged and de-duplicated in code. The evidence sent to Jev is the merged Brief plus the text of the cited sections, in document order, until the budget is reached.
- **FR-8 Review step:** before any decision call, the user sees the Brief (and, in Mode B, the sections with citations) and can edit any field, add or remove items, and then confirm. Nothing reaches the decision pass without this confirmation.

**Decision pass**
- **FR-9 Rule layer (code):**
  1. **cloud-platform** and **backend-platform** are decided first (by Jev, from the skill's tables).
  2. **Variant rows** are then selected by rules:
     - REST API and Testing: `.NET` or `Node` row, by backend-platform
     - IaC: Azure or Hetzner row, by cloud-platform; both rows if hybrid
     - Relational DB: budget row when budget = tight or cloud = Hetzner, otherwise enterprise
     - Auth: a Jev Choice over the skill's Auth Decision Tree app types (internal enterprise / B2B SaaS / B2C / M2M) selects enterprise vs B2C, and the tree's own guidance is added to the question context
  3. Rules only select or exclude rows. They never pick a technology.
- **FR-10 Gates:** one Jev call with three Nouls on the confirmed evidence:
  - `is_technical_request`
  - `has_enough_context`
  - `contains_model_directed_instructions` (injection)

  If `is_technical_request < 0.5`, the pass stops with "Out of remit". If injection ≥ 0.3, **every** decision is routed Needs architect with reason `possible_injection`. If `has_enough_context < 0.5`, the report shows a banner, but decisions still run.
- **FR-11 Applicability:** one Jev call asks a Noul per candidate decision type ("Does this project need a decision about `<type>`?"). Types scoring ≥ 0.5 are applicable. Types scoring < 0.5 are listed in the report as "Not applicable (p=…)" and are not decided.
- **FR-12 Decisions:** for each applicable type, Jev is asked:
  - one **Choice** over the options. Adopt and trial options are always included; a hold option is included only if it matches a `mentioned_technologies` alias.
  - one **Score** per (non-hold option × criterion), on a 5-level rubric defined per criterion.

  The criteria are: `nfr_fit`, `team_skill_fit`, `cost_fit`, `bistec_alignment`, `lock_in` (inverted: less lock-in scores higher), and `security_posture`. Questions are batched up to `max_questions_per_call` (default 64), with at most 4 calls in flight.
- **FR-13 Precedent:** up to 3 of the most recent **Accepted** decisions of the same type (from the local store) are added to that type's state as `bistec_precedent`: the option chosen and a one-line context summary. They count against the budget, and precedent is dropped first when over budget.
- **FR-14 Composition and routing (code):**
  - `composite(option) = Σ_c w_c · score_c / 4`, divided by `Σ w_c`, giving a value in [0, 1]
  - `agree = argmax(composite) == choice`
  - `margin` = the gap between the top two composites
  - Route **Proposed** only if all of these hold: `confidence ≥ threshold`, `agree`, `margin ≥ min_margin`, the choice is not on hold, and there was no injection flag.
  - Otherwise route **Needs architect**, listing every rule that failed as a reason code: `low_confidence`, `disagreement`, `close_margin`, `hold_option`, `possible_injection`.
- **FR-15 Progress:** the pipeline sends progress events for each stage (brief, gates, applicability, decisions k/n, done), and the UI shows them. A failed stage shows its error and a Retry button that re-runs only from the failed stage onward.

**Report, approval, export**
- **FR-16 Report screen:**
  - **Summary:** the Brief, applicable and not-applicable counts, Proposed and Needs-architect counts, and total Jev cost (the sum of `usage.cost`) with input tokens.
  - **One card per decision:** chosen option with its ring badge; a probability bar per option; a confidence meter; a heatmap of criterion scores (option × criterion); composite scores; route and reason codes; cited sections (Mode B); the Jev model snapshot.
- **FR-17 Golden-set calibration:** `cargo run --bin calibrate` runs `golden/*.yaml` (≥10 past BISTEC decisions, each with an expected option) against the live API. It reports accuracy, accuracy on Proposed decisions, and how many decisions went to Needs architect, for a grid of thresholds. It is not part of CI.
- **FR-18 Approval:** each decision has **Accept**, **Override** (pick a different option; reason required), and **Reject** (reason required). Each action records the reviewer name (from Settings, which must be non-empty), a UTC timestamp, the action, the option, and the reason. The review history is append-only: a later review supersedes an earlier one but never deletes it. The decision's ADR status follows the latest review (Proposed (AI) → Accepted / Accepted (override) / Rejected).
- **FR-19 ADR export:** exports to a folder the user picks. It writes `ADR-NNN-<slug>.md` per decision, using the `bistec-architect` ADR template sections (Status / Date / Context / Decision / Bistec Alignment / Alternatives Considered / Consequences / Cost Analysis), plus a **Review** section (the full review history) and an **Evidence** footer (model snapshot, request hash, gate values). NNN continues from the highest existing `ADR-###` number in that folder. **Unreviewed decisions export with `Status: Proposed (AI) — pending approval`.** Cost Analysis shows the budget band and the cost-fit scores, and says `Monthly estimate: to be completed by architect`.
- **FR-20 Report export:** exports the whole report as Markdown and as a self-contained HTML file (inline CSS, no network), and offers a **Print / Save as PDF** option through the webview print dialog.
- **FR-21 History:** sessions are stored in local SQLite: input, brief, decisions, reviews, and Jev call usage. The home screen lists them, and opening one restores its report exactly, **without re-calling Jev**.
- **FR-22 Catalogue check:** `cargo run --bin catalog-check -- <path/to/SKILL.md>` parses the skill's Technology Selection Matrix and reports rows or options that were added, removed, or moved between rings, compared with `catalog/decision-types.yaml`. It exits non-zero on any drift.

### Non-Functional Requirements

- **NFR-1 Platforms:** builds and runs on macOS (Apple Silicon and Intel) and Windows 10/11 x64. CI builds both.
- **NFR-2 Offline behaviour:** everything except Jev calls works offline: history, reports, export, and brief editing. When Jev can't be reached, the error names the failing stage and nothing is lost.
- **NFR-3 Secrets:** the API key never appears in logs, the SQLite store, config files, IPC responses, or exports. Any logging of request headers redacts `Authorization`.
- **NFR-4 Resilience:** Jev calls use a 60 s timeout and retry up to 3 times with exponential backoff on 429/5xx/timeouts. Ollama calls use a 120 s timeout per request.
- **NFR-5 Determinism in code:** given the same Jev answers, composition, routing, rules, and rendering produce byte-identical output. They are pure functions with unit tests.
- **NFR-6 Performance:** in Mode A, from confirming the brief to the report appearing takes ≤ 60 s for 27 decision types on a normal connection, excluding Jev latency above 10 s per call.
- **NFR-7 Stack:** follows the BISTEC defaults where they apply: React + TypeScript (strict) + Tailwind + shadcn/ui, Zod for validation on the UI side, Vitest, Playwright, and a GitHub Actions CI.
- **NFR-8 Bundle:** the app does not bundle the model. Ollama is a documented prerequisite.

## Acceptance Criteria

- **AC-1** `pnpm install && pnpm tauri build` produces an app bundle on macOS. The CI workflow `.github/workflows/ci.yml` has a matrix `[macos-latest, windows-latest]` that runs `cargo test`, `pnpm test`, and `pnpm tauri build --no-bundle`. (NFR-1)
- **AC-2** `catalog/decision-types.yaml` contains all 25 matrix rows plus `cloud-platform` and `backend-platform`, with each option's ring matching the skill. `cargo run --bin catalog-check -- <SKILL.md>` exits 0 against the current skill, and exits non-zero on a fixture skill with one row edited. (A1, A2, FR-22)
- **AC-3** A unit test with a fake credential store proves `set_api_key` stores the key, `has_api_key` returns true, and no IPC command returns the key's value. A grep test over the SQLite file and log output after a run finds no key substring. (FR-2, NFR-3)
- **AC-4** The Jev client, tested against a mock HTTP server, sends `model`, `state`, and `questions` exactly as specified. It parses Noul, Choice (with `probabilities` and `confidence`), and Score (with `score`, `probabilities`, `legend`) answers, and `usage.cost`. It retries a 429 and then succeeds, and it gives up after 3 retries with a typed error. (A3, NFR-4)
- **AC-5** Mode A, with a fake local model returning fixed JSON, produces a Brief that passes schema validation. Malformed JSON causes exactly one retry, and a second failure surfaces a `BriefExtractionFailed` error. (FR-5)
- **AC-6** Fixture documents (`.pdf`, `.docx`, `.md`, `.txt`) each parse into sections with ids `S1..Sn` and non-empty text. A fixture over the budget takes the *large* path (per-section extraction and merge, with every Brief item tagged with a section id). A fixture under the budget takes the *small* path (Jev context Choices, each with `not_stated`). (FR-6, FR-7)
- **AC-7** The UI blocks the decision pass until the Brief is confirmed. An edited Brief field is what gets sent in the next Jev request's `state`, as asserted with the mock client. (FR-8)
- **AC-8** Rule-layer unit tests:
  - backend = `.NET` selects the REST API (.NET) and Testing (.NET) rows, never the Node rows
  - budget = tight selects the Relational DB (budget) row
  - cloud = hybrid selects both IaC rows
  - no rule outputs a technology choice (FR-9)
- **AC-9** Gate tests:
  - `is_technical_request = 0.2` stops the pass with "Out of remit" and makes no applicability or decision calls
  - injection = 0.4 routes every decision Needs architect with `possible_injection` (FR-10)
- **AC-10** Composition and routing table tests cover each reason code on its own and in combination. With equal weights, composite values match figures calculated by hand to 1e-9. A hold option is never Proposed. (FR-14, NFR-5)
- **AC-11** Decision-question builder tests:
  - a hold option appears in the Choice only when a `mentioned_technologies` alias matches it
  - Score questions have exactly 5 levels
  - question batches never exceed `max_questions_per_call`
  - no Choice has more than 255 options (FR-12)
- **AC-12** Precedent: with 5 accepted decisions of a type in the store, the state includes the 3 most recent. When the state is over budget, precedent is removed before any evidence is. (FR-13)
- **AC-13** Approval:
  - Accept, Override, and Reject all require a non-empty reviewer name
  - Override and Reject require a reason
  - the review history is append-only (after two reviews, both rows are still in the store)
  - the ADR status follows the latest review (FR-18)
- **AC-14** ADR export:
  - golden-file tests render ADRs identical to fixtures
  - numbering continues after an existing `ADR-007-x.md` (the next file is `ADR-008-…`)
  - unreviewed decisions render `Status: Proposed (AI) — pending approval`
  - the Cost Analysis section contains `to be completed by architect` (FR-19)
- **AC-15** The report exports as Markdown and self-contained HTML. The HTML contains no `http(s)://` references to scripts or stylesheets. (FR-20)
- **AC-16** Reopening a session from history renders the same report with **zero** Jev calls, as asserted with a counting mock. (FR-21)
- **AC-17** The first-run notice blocks the first Jev call until it is acknowledged, and the acknowledgement persists across restarts. (FR-3)
- **AC-18** A Playwright smoke test on the web build, with a mocked Tauri IPC, walks through: Describe → confirm Brief → progress → Report with ≥1 card → Accept → export button enabled. (FR-15, FR-16)
- **AC-19** `golden/` contains ≥10 cases, and `cargo run --bin calibrate -- --dry-run` validates them without calling the network. (FR-17)
- **AC-20** Health check: the model-missing state shows the exact `ollama pull hf.co/openbmb/MiniCPM5-2B-GGUF:Q4_K_M` command (reflecting the configured model). (FR-4)

## Edge Cases

- **Empty or tiny input** (< 20 chars): the Describe button is disabled with a hint.
- **Scanned or image-only PDF:** zero extractable text → error "This PDF has no text layer (OCR is not supported in v1)".
- **A document well over the budget even after extraction:** the merged Brief is truncated by requirement priority (constraints, then NFRs, then requirements), and the report shows a "truncated" banner.
- **Ollama not running / model not pulled:** Mode A is disabled with the health-check message. Mode B small-path still works, because it doesn't need MiniCPM.
- **Jev returns an answer key we didn't ask for, or leaves one out:** the stage fails with a typed error. Nothing is guessed.
- **Choice probabilities tie at the top:** the result is treated as `disagreement` if the composite argmax differs, and always as `close_margin` if the composites also tie.
- **Every option for a type is on hold** (can't happen with the current skill, but must be safe): the type is skipped with the reason `no_eligible_options`.
- **Cost:** `usage.cost` missing → cost is shown as "n/a" for that call. It is never estimated.
- **Two reviewers on the same machine:** the reviewer name is taken from Settings at the moment of the action. Changing it later doesn't rewrite history.
- **The export folder has ADR files with non-standard names:** only `ADR-(\d{3,})-` prefixes count towards numbering.

## Dependencies

- Ollama ≥ 0.5 (structured outputs through the JSON-schema `format`) with `hf.co/openbmb/MiniCPM5-2B-GGUF:Q4_K_M` pulled.
- An OpenRouter account and API key with credit. Jev `typesafe/jev-1.13`.
- Toolchain: Rust stable (Tauri 2 needs ≥1.77), Node ≥20, pnpm, and `@tauri-apps/cli` v2 as a devDependency (the globally installed `cargo-tauri` is 1.5 and is **not** used).
- Source content: `bistec-architect` `SKILL.md` (synced skill), converted once into `catalog/`.

## Notes

- Grounding: TypeSafe docs (`/api`, `/confidence`, `/model-jaggedness/jev-1.13`, `/concepts/state`) and the OpenRouter Jev guide and tutorial, fetched 2026-09-23. The "jaggedness" rules shape FR-12's question wording: literal instructions, criteria for every option, no arithmetic, filtered state.
- The skill mentions `references/solution-architecture-template.md`, which is not present in the synced skill. It isn't needed for v1.
