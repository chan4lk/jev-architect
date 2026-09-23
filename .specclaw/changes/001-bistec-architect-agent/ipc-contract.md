# IPC contract: Rust commands ↔ React UI

This is the binding contract between **T10** (Rust `commands.rs`) and **T11–T13** (the UI). Both sides implement exactly these shapes.

**Naming conventions:**
- JSON field names are **snake_case** (the serde default in the Rust DTOs).
- Tauri 2 command arguments are passed from JS in **camelCase**, e.g. `invoke('review', { decisionId, action, optionId, reason })`. Tauri maps them to the snake_case Rust parameters.

**Errors:** every command rejects with `AppError = { code: string, message: string, stage?: string }`. The codes are:
`no_api_key`, `data_notice_required`, `not_found`, `validation`, `brief_extraction_failed`, `jev`, `local_model`, `document`, `store`, `io`, `out_of_remit`, `internal`.

## DTOs

```ts
type Ring = 'adopt' | 'trial' | 'hold'
type Route = 'proposed' | 'needs_architect'
type ReasonCode = 'low_confidence' | 'disagreement' | 'close_margin' | 'hold_option' | 'possible_injection'
type AdrStatus = 'proposed_ai' | 'accepted' | 'accepted_override' | 'rejected'
type ReviewAction = 'accept' | 'override' | 'reject'

Settings = { jev_model, jev_base_url, ollama_base_url, ollama_model: string, state_token_budget, confidence_threshold, min_margin, max_questions_per_call, max_concurrent_calls: number, weights: Record<string, number>, reviewer_name: string }
// weights: {} means "use the catalogue defaults"; get_settings returns the effective (merged) weights.

Criterion = { id, name: string, weight: number }            // catalogue default weight
ContextAssessment = { scale: 'small'|'medium'|'large'|'unknown', budget: 'tight'|'moderate'|'enterprise'|'unknown', timeline: 'urgent'|'normal'|'long_term'|'unknown', team_size: 'solo_pair'|'small'|'large'|'unknown', compliance: ('soc2'|'gdpr'|'hipaa'|'industry_specific')[], data_sensitivity: 'public'|'internal'|'confidential'|'restricted'|'unknown' }
BriefItem = { text: string, sources: string[] }              // sources = section ids "S3"
Brief = { summary: string, context: ContextAssessment, requirements, nfrs, constraints, team_skills: BriefItem[], mentioned_technologies: string[] }
Section = { id: string, heading: string | null, text: string, tokens: number }

SessionSummary = { id, created_at /* RFC3339 */, title, mode: 'describe'|'upload', stage: string, decision_count: number, needs_architect_count: number, unreviewed_count: number }
Session = { id, created_at, title, mode, input_text: string|null, doc_name: string|null, stage: string, error: string|null, brief_confirmed_at: string|null }
// stage ∈ 'brief' | 'review' (brief ready, awaiting confirm) | 'gates' | 'platforms' | 'applicability' | 'decisions' | 'done' | 'out_of_remit' | 'failed:<stage>'

OptionView = { id, name: string, ring: Ring, description: string }
OptionScore = { option_id: string, criterion_scores: Record<string, number> /* 0..4 */, composite: number /* 0..1 */ }
DecisionResult = { id, session_id, type_id, choice: string, confidence: number, probabilities: Record<string, number>, option_scores: OptionScore[], route: Route, reasons: ReasonCode[], model_snapshot, request_hash: string, cited_sections: string[] }
Review = { id: number, decision_id: string, action: ReviewAction, option_id: string|null, reviewer: string, reason: string|null, at_utc: string }
DecisionView = { decision: DecisionResult, type_name: string, options: OptionView[], reviews: Review[], status: AdrStatus, status_label: string }
NotApplicable = { type_id: string, type_name: string, probability: number }
Gates = { is_technical_request: number, has_enough_context: number, injection: number }
Report = { gates: Gates, decisions: DecisionView[], not_applicable: NotApplicable[], input_tokens: number, cost_usd: number | null, truncated: boolean, criteria: Criterion[] }

SessionView = { session: Session, sections: Section[] | null, doc_path: 'small' | 'large' | null, brief: Brief | null, report: Report | null }
Health = { ollama_reachable: boolean, model_present: boolean, model: string, pull_command: string, openrouter: 'ok' | 'no_key' | 'error', openrouter_error: string | null }
Progress = { session_id: string, stage: string, done: number, total: number, message: string | null }   // event name: "pipeline://progress"
```

## Commands

| invoke name | args (JS camelCase) | resolves |
|---|---|---|
| `get_settings` | – | `Settings` |
| `save_settings` | `{ settings }` | `Settings` |
| `get_criteria` | – | `Criterion[]` |
| `set_api_key` | `{ key }` | `null` |
| `clear_api_key` | – | `null` |
| `has_api_key` | – | `boolean` |
| `data_notice` | – | `{ text: string, acked: boolean }` |
| `ack_data_notice` | – | `null` |
| `health_check` | – | `Health` |
| `start_describe` | `{ text }` | `SessionView` (stage `review` with brief, or rejects `brief_extraction_failed` / `local_model`) |
| `start_upload` | `{ path }` | `SessionView` (stage `review`; `doc_path` set) |
| `get_session` | `{ id }` | `SessionView` |
| `list_sessions` | – | `SessionSummary[]` |
| `update_brief` | `{ id, brief }` | `SessionView` |
| `confirm_brief` | `{ id }` | `SessionView` |
| `run_decisions` | `{ id }` | `SessionView` (stage `done` or `out_of_remit`); emits progress events; needs confirmed brief, API key, and acknowledged notice |
| `retry` | `{ id }` | `SessionView` (resumes from the failed stage) |
| `review` | `{ decisionId, action, optionId?, reason? }` | `DecisionView` (reviewer taken from settings.reviewer_name; rejects `validation` if blank) |
| `export_adrs` | `{ id, dir }` | `string[]` (written paths) |
| `export_report` | `{ id, dir, format: 'md' \| 'html' }` | `string` (written path) |

The UI never receives the API key. `has_api_key` is the only way it can learn anything about the key.
