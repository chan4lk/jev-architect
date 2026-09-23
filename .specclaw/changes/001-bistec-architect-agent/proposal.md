# Proposal: BISTEC Architect — Tauri desktop app (local MiniCPM + Jev decisions)

**Created:** 2026-09-23
**Revised:** 2026-09-23 (rev 3: CLI → Tauri desktop app; local model pre-processing added)
**Status:** ✅ Approved (2026-09-23)

## Problem

BISTEC Global makes technical decisions every week: stack selection, hosting, integration patterns, build-vs-buy, and responses to technical questions in RFPs. Today these decisions depend on whichever senior architect is free. That causes three problems:

- **Inconsistency.** Similar questions get different answers on different projects, and nobody checks them against the company's preferred stack or past decisions.
- **Bottleneck.** Pre-sales and delivery teams wait for architect time before they can write proposals or start work.
- **Lost rationale.** Decisions are rarely written down as ADRs, so the reasoning is lost when people move between projects.

Pre-sales and delivery people need a **desktop tool** where they can type a description of a project or drop in a requirements document, and get back a **report of BISTEC-aligned technology decisions**. The report should show how confident each decision is, flag which ones need a human architect, and export as ADRs.

## Key constraints

**Jev** (`typesafe/jev-1.13`, via OpenRouter; see the [OpenRouter Jev guide](https://openrouter.ai/docs/guides/community/jev) and [TypeSafe docs](https://docs.typesafe.ai)):

- It is a **decision model, not an LLM**. You send a `state` plus typed questions:
  - **Choice:** picks one option, with a probability for each (up to 255 options)
  - **Score:** places the state on a rubric of 2–10 levels
  - **Noul:** the probability that a yes/no statement is true
- It generates no text and cannot come up with options. It is weak at arithmetic and dates, and reads instructions literally. It is not hardened against adversarial content in `state`.
- Its context is **32k tokens**, and accuracy drops when the state is large and full of irrelevant detail. The docs say to filter first.
- API: `POST https://openrouter.ai/api/v1/systemone` (or `/api/alpha/decisions`). You pay per input token; output tokens are free.

**Local model:** MiniCPM5-2B (`openbmb/MiniCPM5-2B-GGUF`) is a small text model that runs on-device. It is good for extraction and summarising. It is **not** trusted to make the architectural decision; that is Jev's job, scored against the BISTEC catalogue.

## Proposed Solution

A **Tauri 2 desktop app** (Rust core with a React UI) that runs a pipeline with three roles: **local model = understand, Jev = decide, code = control.**

### Two input modes

**Mode A: Describe (free text).** The user types something like *"Client portal for a Sri Lankan insurer, ~20K users, needs SSO with their M365, tight budget, 6-week timeline, team is .NET-heavy"*.
1. **MiniCPM (local) breaks it down** into a structured brief: the six `bistec-architect` Context Assessment dimensions (scale, budget, timeline, team size, compliance, data sensitivity), atomic requirements, NFRs, and constraints. Output is JSON constrained by a grammar or schema. Missing dimensions are marked `unknown` rather than guessed.
2. The UI shows the brief **for the user to confirm or edit** before anything is sent to Jev. This is the human check on what the small model extracted.

**Mode B: Upload a requirements document** (PDF, DOCX, MD, TXT).
1. The document is converted to text in the Rust core and split into sections.
2. **Size decides the path** (decided 2026-09-23):
   - **Small document** (within the Jev token budget): the whole document text is sent to Jev as `state`, split into sections so answers can cite them.
   - **Large document** (over the budget): **MiniCPM builds the brief** section by section (extraction and summarisation). Jev then receives the brief and the most relevant cited passages, not the raw document.
3. In both cases the user sees the brief or the document sections **with citations** before the decision pass, and can edit them.

### The decision pass (both modes)

3. **Applicability fan-out (Jev Noul):** one question per decision type in the catalogue, asking "Does this project need a decision on `<message queue / document DB / real-time / …>`?". Decision types that don't apply are dropped. This uses TypeSafe's "speculative fan-out" pattern.
4. **Decisions (Jev, one batched call):** for each applicable decision type:
   - **Choice** over the catalogue options: adopt/trial options, plus hold options only when the brief or document names them
   - **Scores** per criterion: NFR fit, team skill, cost fit against the budget band, BISTEC alignment, lock-in, and security
5. **Code combines and routes:**
   - A weighted composite, using weights from config
   - An agreement check between the composite ranking and Jev's Choice
   - Confidence gating: a confident, agreeing result is **Proposed**; low confidence, disagreement, or a hold option is **Needs architect**
6. **Rule layer (code, not Jev):** the `bistec-architect` auth decision tree and stack dependencies. For example, if backend = .NET, the "REST API (.NET)" row applies. These run before and between the Jev calls.

### Report UI

- **Summary:** project brief, number of decisions, how many are Proposed vs Needs architect, and estimated Jev cost (input tokens × price)
- **One card per decision:**
  - the chosen option, with an adopt/trial/hold badge
  - a probability bar for every option
  - a confidence meter
  - a heatmap of criterion scores
  - the source passages (Mode B), and the gate or disagreement reasons
- **Actions per decision:** Accept / Override (pick another option, with a reason) / Reject, each recorded with the reviewer's name
- **Export:** one ADR per decision using the `bistec-architect` ADR template (Markdown), and the full report as PDF/Markdown
- **History:** past sessions saved locally (SQLite). Accepted decisions become precedent for future sessions.

### Architecture

| Layer | Choice | Why |
|---|---|---|
| Shell | **Tauri 2** | Small native binary, runs on macOS and Windows, Rust core |
| UI | React + TypeScript + Tailwind + shadcn/ui (Vite) | BISTEC frontend defaults (Next.js isn't needed inside Tauri) |
| Core | Rust: document parsing, the pipeline, the Jev HTTP client (`reqwest`), SQLite (`sqlx`/`rusqlite`) | Jev's API is plain JSON, so no SDK is needed; keeps all of it in one process |
| Local model | **Ollama** (`hf.co/openbmb/MiniCPM5-2B-GGUF`) over its localhost HTTP API; the model name is configurable | Already installed on dev machines; bundling llama.cpp as a sidecar is a later step |
| Secrets | OpenRouter key in the OS keychain (Tauri stronghold/keyring plugin) | The key is never stored in plain config files or sent to the UI |
| Knowledge base | Catalogue, criteria, and ADR template converted from the `bistec-architect` skill into versioned YAML bundled with the app | Works offline; `catalog check` flags drift from the skill |

This **replaces the Python CLI** in rev 2. The earlier "Python" decision doesn't apply to a Tauri app. The pipeline logic moves to Rust, and there is no Python runtime to ship.

## Scope

### In Scope
- A Tauri 2 app scaffold with macOS and Windows builds (both are v1 targets), with a React UI using BISTEC's frontend stack
- Settings screen: OpenRouter key (stored in the keychain), Jev model (pinned `typesafe/jev-1.13` by default), Ollama URL and model, confidence thresholds, criterion weights
- Mode A: free text → MiniCPM brief → editable brief screen
- Mode B: document upload (PDF, DOCX, MD, TXT) → sections → whole to Jev if small, MiniCPM brief if large → review screen with citations
- A Jev client for batched Choice, Score, and Noul calls, with retries, a token budget guard, and cost logging
- Applicability fan-out, decisions, composite scoring, confidence routing, and the rule layer
- Report UI (summary, decision cards, accept/override/reject), ADR Markdown export, report export
- A local session history and precedent store (SQLite)
- Knowledge base YAML converted from `bistec-architect`, plus `catalog check`
- Tests: Rust unit tests with mocked Jev and Ollama; a golden set of about 10 past BISTEC decisions for calibration; UI smoke test (Playwright via `tauri-driver`, or a web-only build)

### Out of Scope
- An LLM-written rationale; ADRs contain evidence only (rev-2 decision, still applies)
- Letting MiniCPM make or rank decisions
- Bundling the model inside the app installer (v1 requires Ollama)
- Dollar cost estimates (the skill's cost table is left for the architect to fill in)
- Multi-user, sync, or server mode; Teams/Slack/Copilot integration; Apex decision-log sync
- Scanned or image-only PDFs (would need OCR; possible later with `glm-ocr` or MiniCPM-V)
- Auto-update and code signing / notarisation for distribution

## Impact

- **Size:** architectural (spike / bounded / architectural). This is a new desktop application with three runtimes (Rust core, web UI, local model server) and one external API.
- **Files affected:** ~50–80 (estimated)
- **Complexity:** large
- **Risk:** medium–high:
  - A 2B model extracting the brief can miss or invent constraints. Mitigation: the user edits the brief, and missing values are marked `unknown`.
  - Requirements documents contain client data and are sent to OpenRouter/TypeSafe without redaction (an accepted decision). Mitigation: a first-run notice, and the user reviews the content before sending.
  - Jev is not hardened against adversarial text in documents. Mitigation: an injection Noul gate, and a human reviews every decision.

## Resolved Decisions

- **Evidence-only ADRs for v1** (2026-09-23)
- **The `bistec-architect` skill** is the base for the catalogue, context schema, and ADR template (2026-09-23)
- **Tauri desktop app with a report UI**, a local model for pre-processing, and Jev for decisions (2026-09-23, rev 3). *This replaces the rev-2 "Python CLI" decision.*
- **All 26 Technology Selection Matrix rows** are in v1 scope (2026-09-23)
- **macOS and Windows** are both v1 targets (2026-09-23)
- **Local model: MiniCPM5-2B** (`openbmb/MiniCPM5-2B-GGUF`, via Ollama) (2026-09-23)
- **Mode B:** a small document goes to Jev whole; a large one goes through a MiniCPM brief first (2026-09-23)
- **No redaction** of client or person names in v1. The user accepts that document content is sent to OpenRouter/TypeSafe; the Settings screen and first-run notice say so (2026-09-23)
- **Criterion weights:** drafted by us from the `bistec-architect` preferences, bundled as defaults, and editable in Settings (2026-09-23)
- **Approval required:** every decision needs a named reviewer to Accept, Override, or Reject it. Each action is stored with the reviewer's name, a timestamp, and a reason (required for Override/Reject), and appears in exported ADRs and the report. An ADR is only marked `Accepted` once it has been approved (2026-09-23)
- **Jev pinned to `typesafe/jev-1.13`.** Confidence thresholds are calibrated against this version; upgrading is a deliberate change (2026-09-23)

## Open Questions

_None; all questions were resolved on 2026-09-23._
