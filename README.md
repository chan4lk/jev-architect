# BISTEC Architect

A desktop app that turns a project description or a requirements document into
a set of proposed technology decisions for a BISTEC engagement — with a human
architect always in the loop before anything is treated as accepted.

**Local MiniCPM understands, Jev decides, code controls.** A local Ollama
model (MiniCPM) reads your project description or document and extracts a
structured brief. A hosted reasoning model ("Jev", `typesafe/jev-1.13` on
OpenRouter) answers targeted questions about that brief — cloud platform,
backend, database, auth, and the rest of the BISTEC technology catalogue.
Deterministic Rust code combines Jev's answers with BISTEC's own rules into a
composite score and a routing decision; it never lets a model do arithmetic or
pick a technology on its own. **Human approval is required**: every decision
starts as `Proposed (AI) — pending approval` and only becomes accepted,
overridden, or rejected once an architect reviews it in the app.

## Prerequisites

- **Rust** stable, ≥1.77 (Tauri 2 requirement; `src-tauri/Cargo.toml` pins
  `rust-version = "1.77.2"`).
- **Node** ≥20 and **pnpm**.
- **Ollama** ≥0.5, with the local model pulled:

  ```sh
  ollama pull hf.co/openbmb/MiniCPM5-2B-GGUF:Q4_K_M
  ```

- An **OpenRouter** account and API key with credit, for the Jev model
  (`typesafe/jev-1.13`). Set the key from inside the app (Settings → OpenRouter
  API key) — it's stored in the OS keychain (macOS Keychain / Windows
  Credential Manager via the `keyring` crate) and never appears in logs, the
  local SQLite store, config files, or exports.

The app doesn't bundle either model: Ollama and an OpenRouter key are
documented prerequisites, not something the installer sets up for you.

## Data notice

Before the first decision run, the app shows a data-handling notice that must
be acknowledged:

- The text of your project brief and the decision questions built from it are
  sent **unredacted** to OpenRouter/TypeSafe (the Jev model) to produce
  architecture decisions.
- Local brief extraction (Mode A, and small-document Mode B) runs against your
  local Ollama instance and never leaves the machine.
- Nothing else leaves the machine. The OpenRouter API key is never sent back
  to the UI, logged, or written to disk outside the OS keychain.

## Getting started

```sh
pnpm install
pnpm tauri dev
```

This starts the Tauri dev window with the Vite dev server backing the UI. Set
the OpenRouter key and acknowledge the data notice from Settings before
running your first decision pass; Ollama must be running with the model
pulled for Mode A (describing a project in your own words).

## Build

```sh
pnpm tauri build
```

Produces a native app bundle for the current platform. CI
(`.github/workflows/ci.yml`) builds and tests on a `[macos-latest,
windows-latest]` matrix on every push/PR, running `pnpm test`, `cargo test`,
and `pnpm tauri build --no-bundle` — Windows is only exercised there, not on a
local Windows machine.

## Tests

```sh
pnpm test                                          # frontend unit tests (Vitest)
cargo test --manifest-path src-tauri/Cargo.toml    # Rust unit/integration tests
pnpm e2e                                           # Playwright smoke test (this change)
```

`pnpm e2e` builds the web bundle and serves it with `vite preview`; it runs
against the in-memory mock IPC (`src/lib/ipc-mock.ts`), which the app uses
automatically whenever it isn't running inside a Tauri window, so no running
Ollama or OpenRouter key is needed for the smoke test.

## Catalogue maintenance

The BISTEC technology catalogue (`catalog/decision-types.yaml`,
`catalog/criteria.yaml`, `catalog/rules.yaml`) is converted from the
`bistec-architect` skill's Technology Selection Matrix. When the skill
changes, check the catalogue hasn't drifted from it:

```sh
cargo run --manifest-path src-tauri/Cargo.toml --bin catalog-check -- <path to bistec-architect SKILL.md>
```

Exits non-zero (and reports the specific rows/options) if a technology was
added, removed, or moved between rings without the catalogue being updated.

## Calibration

Confidence thresholds, the minimum margin, and criterion weights ship as
**uncalibrated defaults** — the app labels them as such in Settings until this
has been run. `calibrate` replays `golden/*.yaml` (real past BISTEC decisions,
each with an expected option) against a grid of thresholds and reports
accuracy, accuracy on `Proposed` decisions, and how often decisions would be
routed to Needs architect:

```sh
# offline validation of the golden set, no network calls
cargo run --manifest-path src-tauri/Cargo.toml --bin calibrate -- --dry-run

# live run against the real Jev API (needs credit)
OPENROUTER_API_KEY=sk-or-... cargo run --manifest-path src-tauri/Cargo.toml --bin calibrate
```

`calibrate` is not part of CI — it's a release/tuning step, not a correctness
check.

## How decisions are made

```
input ─► brief ─► (architect confirms) ─► gates ─► platforms ─► applicability ─► decisions ─► report
```

- **Brief**: MiniCPM (Mode A) or Jev (Mode B) extracts a structured brief —
  summary, context assessment, requirements, NFRs, constraints, team skills,
  and mentioned technologies — from your description or document. Nothing is
  sent to Jev until you confirm the brief.
- **Gates**: three checks against the brief — is this actually a technology
  decision request, is there enough context, and is there a possible prompt
  injection. A failing "is this a technical request" gate stops the pass
  entirely ("Out of remit").
- **Platforms**: cloud platform, backend platform, and (if relevant) the auth
  application type are decided first, since later rules key off them.
- **Applicability**: each of the 27 decision types in the catalogue is asked
  whether it applies to this brief at all.
- **Decisions**: for every applicable type, Jev is asked to choose an option
  and score every option against BISTEC's criteria; deterministic Rust code
  composes those scores into a single value per option and decides the route.

A decision is only routed **Proposed** if Jev's choice agrees with the
highest-scoring option, confidence clears the threshold, the margin over the
runner-up is wide enough, the choice isn't a "hold" (avoid) option, and no
possible prompt injection was flagged. Otherwise it's routed **Needs
architect**, tagged with every reason that applied:

| Reason code | Meaning |
|---|---|
| `low_confidence` | Jev's own confidence was below the threshold |
| `disagreement` | the highest-scoring option isn't the one Jev chose |
| `close_margin` | the top two composite scores were too close |
| `hold_option` | the chosen option is on BISTEC's "avoid" ring |
| `possible_injection` | the gates stage flagged possible prompt injection |

Every decision — Proposed or Needs architect — still requires an architect to
Accept, Override (with a reason), or Reject it before it can be exported as an
ADR with a status other than `Proposed (AI) — pending approval`.

## Project layout

```
jev-architect/
├─ src/                    React UI (TS strict, Tailwind, shadcn/ui)
│  ├─ lib/ipc.ts            typed wrappers over Tauri's invoke()/listen() — the only file that imports @tauri-apps/*
│  ├─ lib/ipc-mock.ts        in-memory mock used outside Tauri (dev-in-browser, Vitest, Playwright)
│  ├─ lib/schemas.ts         Zod mirrors of the Rust DTOs
│  ├─ screens/               Home | Brief | Run | Report | Settings
│  └─ components/            DecisionCard, ProbBars, ConfidenceMeter, ScoreHeatmap, ReviewDialog, DataNotice, HealthPanel, …
├─ e2e/smoke.spec.ts        Playwright smoke test (AC-18), see `pnpm e2e` above
├─ catalog/                  the BISTEC technology catalogue, bundled into the binary
│  ├─ decision-types.yaml    27 decision types (25 matrix rows + cloud-platform + backend-platform)
│  ├─ criteria.yaml          6 scoring criteria and default weights
│  ├─ rules.yaml             variant-row selection rules (data, not code)
│  └─ adr-template.md        the ADR template used by export
├─ golden/*.yaml             calibration cases for `calibrate`
├─ .github/workflows/ci.yml  macOS + Windows CI matrix
└─ src-tauri/
   ├─ src/commands.rs        the Tauri command surface the UI calls
   ├─ src/pipeline.rs        stage orchestration, budget fitting, precedent, progress events
   ├─ src/scoring.rs         composite score, agreement, margin, routing (pure functions)
   ├─ src/jev.rs             the Jev (OpenRouter) HTTP client
   ├─ src/ollama.rs          the local Ollama client
   ├─ src/docs.rs            PDF/DOCX/Markdown/text extraction
   ├─ src/store.rs           SQLite storage (sessions, decisions, reviews, Jev call usage)
   ├─ src/secrets.rs         the OS-keychain-backed API key store
   ├─ src/render.rs          ADR and report rendering (Markdown / self-contained HTML)
   └─ src/bin/catalog_check.rs   the catalogue-drift checker above (`calibrate` is a companion binary landing alongside it)
```

## Known limitations

- **No OCR.** A scanned or image-only PDF with no extractable text layer is
  rejected with an error; it is not run through OCR.
- **No dollar cost estimates.** ADR exports show the budget band and cost-fit
  scores, but the actual monthly cost is left for the architect to fill in —
  the app never estimates infrastructure cost in dollars.
- **Windows is built in CI only.** The Windows leg of the CI matrix builds and
  runs the test suite; there is no local Windows QA pass as part of this
  project.
- **Unsigned builds may trigger keychain prompts.** Local (non-CI) macOS
  builds are not signed or notarized, so the OS may prompt for keychain access
  the first time the app stores or reads the OpenRouter API key.
