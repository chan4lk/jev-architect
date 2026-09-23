//! Tests for the command surface (`bistec_architect::commands`) without a
//! Tauri runtime: in-memory `Store`, `MemoryStore` secrets, `FakeJev` and
//! `FakeLocalModel`. Covers AC-3, AC-13, AC-17, the no-key gate, export,
//! and writes the cross-language contract fixtures that
//! `src/lib/contract.test.ts` parses with the UI's Zod schemas.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use bistec_architect::commands::{self as cmd, AppError, AppState, ClientFactory, LiveClients};
use bistec_architect::jev::{
    Answer, DecisionClient, DecisionRequest, DecisionResponse, FakeJev, JevError, Question, Usage,
};
use bistec_architect::model::catalog::Catalog;
use bistec_architect::model::review::ReviewAction;
use bistec_architect::model::settings::Settings;
use bistec_architect::ollama::{FakeLocalModel, LocalModel, LocalModelError};
use bistec_architect::pipeline::{NoopSink, RecordingSink};
use bistec_architect::secrets::MemoryStore;
use bistec_architect::store::Store;
use serde::Serialize;
use serde_json::{json, Value};

const KEY: &str = "sk-or-v1-TEST-SECRET-KEY-0123456789";
const DESCRIPTION: &str = "We need a customer self-service portal for an insurer, built by a .NET team on Azure.";

// ---- fakes ---------------------------------------------------------------

type Handler = Box<dyn Fn(&DecisionRequest) -> Result<DecisionResponse, JevError> + Send + Sync>;

/// Answers any request: Nouls 0.9 (injection and compliance 0.0), Choices
/// pick the first option, Scores 4 for the chosen option and 1 otherwise.
fn answer(req: &DecisionRequest) -> DecisionResponse {
    let first = |q: Option<&Question>| match q {
        Some(Question::Choice { criteria, .. }) => criteria.keys().next().cloned(),
        _ => None,
    };
    let answers = req
        .questions
        .iter()
        .map(|(key, q)| {
            let a = match q {
                Question::Noul { .. } => Answer::Noul {
                    noul: if key.starts_with("gate__injection") || key.starts_with("context__compliance__") {
                        0.0
                    } else {
                        0.9
                    },
                },
                Question::Choice { criteria, .. } => {
                    let chosen = criteria.keys().next().unwrap().clone();
                    let others = (criteria.len().max(2) - 1) as f64;
                    Answer::Choice {
                        probabilities: criteria
                            .keys()
                            .map(|k| (k.clone(), if *k == chosen { 0.8 } else { 0.2 / others }))
                            .collect(),
                        choice: chosen,
                        confidence: 0.8,
                    }
                }
                Question::Score { .. } => {
                    let parts: Vec<&str> = key.split("__").collect();
                    let chosen = first(req.questions.get(&format!("choice__{}", parts[1])));
                    Answer::Score {
                        score: if chosen.as_deref() == Some(parts[2]) { 4.0 } else { 1.0 },
                        confidence: 0.9,
                        probabilities: BTreeMap::new(),
                        legend: None,
                    }
                }
            };
            (key.clone(), a)
        })
        .collect();
    DecisionResponse {
        id: None,
        model: "typesafe/jev-1.13-20260901".to_string(),
        provider: None,
        answers,
        usage: Usage {
            input_tokens: 100,
            output_tokens: 10,
            cost: Some(0.001),
        },
    }
}

fn brief_json() -> String {
    json!({
        "summary": "A customer self-service portal for an insurer.",
        "context": {
            "scale": "medium", "budget": "moderate", "timeline": "normal",
            "team_size": "small", "compliance": ["gdpr"], "data_sensitivity": "confidential"
        },
        "requirements": [{"text": "Policyholders track claims", "sources": []}],
        "nfrs": [{"text": "Pages load in under two seconds", "sources": []}],
        "constraints": [],
        "team_skills": [{"text": "The team knows .NET", "sources": []}],
        "mentioned_technologies": [".NET", "Azure"]
    })
    .to_string()
}

/// Injects the fakes, and records the key each Jev client was built with.
struct FakeClients {
    jev: Arc<FakeJev<Handler>>,
    local: Arc<FakeLocalModel>,
    keys: Mutex<Vec<Option<String>>>,
}

impl ClientFactory for FakeClients {
    fn jev(&self, _settings: &Settings, api_key: Option<String>) -> Arc<dyn DecisionClient> {
        self.keys.lock().unwrap().push(api_key);
        self.jev.clone()
    }

    fn local(&self, _settings: &Settings) -> Arc<dyn LocalModel> {
        self.local.clone()
    }
}

fn fake_clients(local_responses: Vec<Result<String, LocalModelError>>) -> FakeClients {
    FakeClients {
        jev: Arc::new(FakeJev::new(Box::new(|req: &DecisionRequest| Ok(answer(req))) as Handler)),
        local: Arc::new(FakeLocalModel::new("hf.co/openbmb/MiniCPM5-2B-GGUF:Q4_K_M", false, local_responses)),
        keys: Mutex::new(Vec::new()),
    }
}

fn app_state() -> AppState {
    AppState {
        catalog: Arc::new(Catalog::bundled().unwrap()),
        store: Arc::new(Store::open_in_memory().unwrap()),
        secrets: Arc::new(MemoryStore::default()),
    }
}

fn set_reviewer(state: &AppState, name: &str) {
    let mut s = cmd::get_settings(state).unwrap();
    s.reviewer_name = name.to_string();
    cmd::save_settings(state, s).unwrap();
}

/// A Mode A session at stage `done`: key stored, notice acked, brief confirmed.
async fn done_session(state: &AppState, clients: &FakeClients) -> String {
    cmd::set_api_key(state, KEY).unwrap();
    cmd::ack_data_notice(state).unwrap();
    let id = cmd::start_describe(state, clients, DESCRIPTION).await.unwrap().session.id;
    cmd::confirm_brief(state, &id).unwrap();
    let view = cmd::run_decisions(state, clients, &id, &NoopSink).await.unwrap();
    assert_eq!(view.session.stage, "done");
    id
}

fn to_json<T: Serialize>(v: &T) -> Value {
    serde_json::to_value(v).unwrap()
}

// ---- AC-3: the key never leaves through IPC --------------------------------

#[tokio::test]
async fn api_key_is_stored_but_never_returned_by_any_command() {
    let state = app_state();
    let clients = fake_clients(vec![Ok(brief_json())]);
    let mut outputs: Vec<Value> = Vec::new();

    assert!(!cmd::has_api_key(&state).unwrap());
    assert_eq!(cmd::set_api_key(&state, "   ").unwrap_err().code, "validation");
    // Clipboard garbage and non-OpenRouter strings are refused, never stored.
    assert_eq!(cmd::set_api_key(&state, "kubesec scan \\").unwrap_err().code, "validation");
    assert_eq!(cmd::set_api_key(&state, "not-an-openrouter-key").unwrap_err().code, "validation");
    assert!(!cmd::has_api_key(&state).unwrap());
    outputs.push(to_json(&cmd::set_api_key(&state, &format!("  {KEY}\n")).unwrap()));
    assert_eq!(state.secrets.get().unwrap().as_deref(), Some(KEY), "stored trimmed");
    outputs.push(to_json(&cmd::has_api_key(&state).unwrap()));
    assert!(cmd::has_api_key(&state).unwrap());

    set_reviewer(&state, "Jane Architect");
    outputs.push(to_json(&cmd::get_settings(&state).unwrap()));
    outputs.push(to_json(&cmd::get_criteria(&state).unwrap()));
    outputs.push(to_json(&cmd::data_notice(&state).unwrap()));
    outputs.push(to_json(&cmd::ack_data_notice(&state).unwrap()));
    outputs.push(to_json(&cmd::health_check(&state, &clients).await.unwrap()));

    let view = cmd::start_describe(&state, &clients, DESCRIPTION).await.unwrap();
    let id = view.session.id.clone();
    outputs.push(to_json(&view));
    outputs.push(to_json(&cmd::update_brief(&state, &id, view.brief.as_ref().unwrap()).unwrap()));
    outputs.push(to_json(&cmd::confirm_brief(&state, &id).unwrap()));
    let sink = RecordingSink::new();
    let done = cmd::run_decisions(&state, &clients, &id, &sink).await.unwrap();
    outputs.push(to_json(&done));
    outputs.extend(sink.events().iter().map(to_json));
    outputs.push(to_json(&cmd::retry(&state, &clients, &id, &NoopSink).await.unwrap()));
    outputs.push(to_json(&cmd::get_session(&state, &id).unwrap()));
    outputs.push(to_json(&cmd::list_sessions(&state).unwrap()));
    let decision_id = done.report.as_ref().unwrap().decisions[0].decision.id.clone();
    outputs.push(to_json(&cmd::review(&state, &decision_id, ReviewAction::Accept, None, None).unwrap()));
    let dir = tempfile::tempdir().unwrap();
    let dir_s = dir.path().to_str().unwrap();
    outputs.push(to_json(&cmd::export_adrs(&state, &id, dir_s).unwrap()));
    outputs.push(to_json(&cmd::export_report(&state, &id, dir_s, "md").unwrap()));
    outputs.push(to_json(&cmd::export_report(&state, &id, dir_s, "html").unwrap()));

    // The Jev client did receive the key (it has to authenticate) …
    assert!(clients.keys.lock().unwrap().iter().any(|k| k.as_deref() == Some(KEY)));
    // … but no command result, event, export or Jev request carries it.
    for out in &outputs {
        assert!(!out.to_string().contains(KEY), "key leaked in {out}");
    }
    for req in clients.jev.requests() {
        assert!(!serde_json::to_string(&req).unwrap().contains(KEY));
    }
    for entry in std::fs::read_dir(dir.path()).unwrap() {
        assert!(!std::fs::read_to_string(entry.unwrap().path()).unwrap().contains(KEY));
    }

    cmd::clear_api_key(&state).unwrap();
    assert!(!cmd::has_api_key(&state).unwrap());
    assert_eq!(state.secrets.get().unwrap(), None);
}

// ---- no key: `no_api_key`, and nothing is sent ------------------------------

#[tokio::test]
async fn run_decisions_without_a_key_is_no_api_key_without_any_call() {
    let state = app_state();
    let clients = fake_clients(vec![Ok(brief_json())]);
    cmd::ack_data_notice(&state).unwrap();
    let id = cmd::start_describe(&state, &clients, DESCRIPTION).await.unwrap().session.id;
    cmd::confirm_brief(&state, &id).unwrap();

    for err in [
        cmd::run_decisions(&state, &clients, &id, &NoopSink).await.unwrap_err(),
        cmd::retry(&state, &clients, &id, &NoopSink).await.unwrap_err(),
    ] {
        assert_eq!(err.code, "no_api_key");
    }
    assert_eq!(clients.jev.call_count(), 0);

    // The live factory without a key builds a client that fails before any
    // network call, and the health check reports `no_key`.
    let jev = LiveClients.jev(&Settings::default(), None);
    let req = DecisionRequest {
        model: "typesafe/jev-1.13".to_string(),
        state: json!({}),
        questions: BTreeMap::new(),
    };
    assert!(matches!(jev.decide(&req).await, Err(JevError::NoApiKey)));
    let health = cmd::health_check(&state, &clients).await.unwrap();
    assert_eq!(to_json(&health)["openrouter"], "no_key");
    assert_eq!(clients.jev.call_count(), 0);
}

#[tokio::test]
async fn small_upload_is_gated_on_notice_and_key() {
    let state = app_state();
    let clients = fake_clients(vec![]);
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/docs/sample.md");
    let path = path.to_str().unwrap();

    let err = cmd::start_upload(&state, &clients, path, &NoopSink).await.unwrap_err();
    assert_eq!(err.code, "data_notice_required");
    cmd::ack_data_notice(&state).unwrap();
    let err = cmd::start_upload(&state, &clients, path, &NoopSink).await.unwrap_err();
    assert_eq!(err.code, "no_api_key");
    assert_eq!(clients.jev.call_count(), 0);
    assert!(cmd::list_sessions(&state).unwrap().is_empty(), "no session is created");

    cmd::set_api_key(&state, KEY).unwrap();
    let view = cmd::start_upload(&state, &clients, path, &NoopSink).await.unwrap();
    assert_eq!(view.session.stage, "review");
    assert_eq!(view.doc_path.as_deref(), Some("small"));
}

// ---- AC-17: the data notice blocks the first Jev call ----------------------

#[tokio::test]
async fn run_decisions_before_the_notice_is_acked_makes_no_jev_call() {
    let state = app_state();
    let clients = fake_clients(vec![Ok(brief_json())]);
    cmd::set_api_key(&state, KEY).unwrap();
    let id = cmd::start_describe(&state, &clients, DESCRIPTION).await.unwrap().session.id;
    cmd::confirm_brief(&state, &id).unwrap();

    assert!(!cmd::data_notice(&state).unwrap().acked);
    let err = cmd::run_decisions(&state, &clients, &id, &NoopSink).await.unwrap_err();
    assert_eq!(err.code, "data_notice_required");
    assert_eq!(clients.jev.call_count(), 0);

    cmd::ack_data_notice(&state).unwrap();
    assert!(cmd::data_notice(&state).unwrap().acked);
    let view = cmd::run_decisions(&state, &clients, &id, &NoopSink).await.unwrap();
    assert_eq!(view.session.stage, "done");
    assert!(clients.jev.call_count() > 0);
}

// ---- local_model_status: Ollama-only, no Jev call --------------------------

#[tokio::test]
async fn local_model_status_reports_ollama_state_and_never_calls_jev() {
    let state = app_state();

    // Model present.
    let present = FakeClients {
        jev: Arc::new(FakeJev::new(Box::new(|req: &DecisionRequest| Ok(answer(req))) as Handler)),
        local: Arc::new(FakeLocalModel::new("hf.co/openbmb/MiniCPM5-2B-GGUF:Q4_K_M", true, vec![])),
        keys: Mutex::new(Vec::new()),
    };
    let status = cmd::local_model_status(&state, &present).await.unwrap();
    assert!(status.ollama_reachable);
    assert!(status.model_present);
    assert_eq!(status.model, "hf.co/openbmb/MiniCPM5-2B-GGUF:Q4_K_M");
    assert_eq!(status.pull_command, "ollama pull hf.co/openbmb/MiniCPM5-2B-GGUF:Q4_K_M");
    assert_eq!(present.jev.call_count(), 0);

    // Model not present (but Ollama reachable).
    let missing = FakeClients {
        jev: Arc::new(FakeJev::new(Box::new(|req: &DecisionRequest| Ok(answer(req))) as Handler)),
        local: Arc::new(FakeLocalModel::new("hf.co/openbmb/MiniCPM5-2B-GGUF:Q4_K_M", false, vec![])),
        keys: Mutex::new(Vec::new()),
    };
    let status = cmd::local_model_status(&state, &missing).await.unwrap();
    assert!(status.ollama_reachable);
    assert!(!status.model_present);
    assert_eq!(missing.jev.call_count(), 0);

    // Ollama itself unreachable.
    let down = FakeClients {
        jev: Arc::new(FakeJev::new(Box::new(|req: &DecisionRequest| Ok(answer(req))) as Handler)),
        local: Arc::new(FakeLocalModel::unreachable("hf.co/openbmb/MiniCPM5-2B-GGUF:Q4_K_M")),
        keys: Mutex::new(Vec::new()),
    };
    let status = cmd::local_model_status(&state, &down).await.unwrap();
    assert!(!status.ollama_reachable);
    assert!(!status.model_present);
    assert_eq!(down.jev.call_count(), 0, "local_model_status never touches Jev");

    // No command result carries the key, and it never made contact with the
    // (never-invoked) Jev client's key-tracking.
    assert!(present.keys.lock().unwrap().is_empty());
    assert!(missing.keys.lock().unwrap().is_empty());
    assert!(down.keys.lock().unwrap().is_empty());
}

// ---- AC-13: review validation and append-only history -----------------------

#[tokio::test]
async fn review_validates_and_appends_history() {
    let state = app_state();
    let clients = fake_clients(vec![Ok(brief_json())]);
    let id = done_session(&state, &clients).await;
    let report = cmd::get_session(&state, &id).unwrap().report.unwrap();
    let view = &report.decisions[0];
    let decision_id = view.decision.id.as_str();
    let other = view.options.iter().find(|o| o.id != view.decision.choice).unwrap().id.clone();

    // Blank reviewer.
    let err = cmd::review(&state, decision_id, ReviewAction::Accept, None, None).unwrap_err();
    assert_eq!(err.code, "validation");
    assert_eq!(err.message, "Set your reviewer name in Settings before approving decisions");

    set_reviewer(&state, "Jane Architect");
    for (action, option, reason) in [
        (ReviewAction::Override, Some(other.clone()), None),
        (ReviewAction::Override, None, Some("cheaper".to_string())),
        (ReviewAction::Reject, None, None),
        (ReviewAction::Reject, None, Some("  ".to_string())),
        (ReviewAction::Override, Some("no-such-option".to_string()), Some("x".to_string())),
    ] {
        let err = cmd::review(&state, decision_id, action, option, reason).unwrap_err();
        assert_eq!(err.code, "validation", "{err:?}");
    }
    assert_eq!(
        cmd::review(&state, "missing", ReviewAction::Accept, None, None).unwrap_err().code,
        "not_found"
    );
    assert!(state.store.reviews_for_decision(decision_id).is_empty());

    let accepted = cmd::review(&state, decision_id, ReviewAction::Accept, None, None).unwrap();
    assert_eq!(to_json(&accepted.status), "accepted");
    assert_eq!(accepted.reviews.len(), 1);
    assert_eq!(accepted.reviews[0].reviewer, "Jane Architect");

    let overridden = cmd::review(
        &state,
        decision_id,
        ReviewAction::Override,
        Some(other.clone()),
        Some("The client already runs it".to_string()),
    )
    .unwrap();
    assert_eq!(to_json(&overridden.status), "accepted_override");
    assert_eq!(overridden.status_label, "Accepted (override)");
    assert_eq!(overridden.reviews.len(), 2, "history is append-only");
    assert_eq!(overridden.reviews[0].action, ReviewAction::Accept);
    assert_eq!(state.store.reviews_for_decision(decision_id).len(), 2);
}

// ---- export ----------------------------------------------------------------

#[tokio::test]
async fn export_writes_adrs_with_numbers_and_reports() {
    // A file-backed store so the test can read `decisions.adr_number` back.
    let db_dir = tempfile::tempdir().unwrap();
    let db_path = db_dir.path().join("architect.sqlite");
    let state = AppState {
        store: Arc::new(Store::open(&db_path).unwrap()),
        ..app_state()
    };
    let clients = fake_clients(vec![Ok(brief_json())]);
    let id = done_session(&state, &clients).await;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("ADR-007-existing.md"), "old").unwrap();
    let dir_s = dir.path().to_str().unwrap();

    let decisions = cmd::get_session(&state, &id).unwrap().report.unwrap().decisions;
    let paths = cmd::export_adrs(&state, &id, dir_s).unwrap();
    assert_eq!(paths.len(), decisions.len());
    let first = PathBuf::from(&paths[0]);
    assert!(first.file_name().unwrap().to_str().unwrap().starts_with("ADR-008-"));
    let text = std::fs::read_to_string(&first).unwrap();
    assert!(text.contains("Proposed (AI) — pending approval"));
    for p in &paths {
        assert!(PathBuf::from(p).is_file());
    }

    let conn = rusqlite::Connection::open(&db_path).unwrap();
    for (i, v) in decisions.iter().enumerate() {
        let n: Option<u32> = conn
            .query_row("SELECT adr_number FROM decisions WHERE id = ?1", [&v.decision.id], |r| r.get(0))
            .unwrap();
        assert_eq!(n, Some(8 + i as u32), "adr number of {}", v.decision.type_id);
    }

    let md = cmd::export_report(&state, &id, dir_s, "md").unwrap();
    assert!(md.ends_with(".md") && PathBuf::from(&md).is_file());
    let html = cmd::export_report(&state, &id, dir_s, "html").unwrap();
    assert!(html.ends_with(".html"));
    assert!(std::fs::read_to_string(&html).unwrap().contains("<html"));
    assert_eq!(cmd::export_report(&state, &id, dir_s, "pdf").unwrap_err().code, "validation");
}

// ---- contract fixtures -------------------------------------------------------

/// Fixtures are committed; they are regenerated only when
/// `UPDATE_CONTRACT_FIXTURES=1` (CI sets it, then runs the Zod contract test).
fn write_fixture<T: Serialize>(name: &str, v: &T) {
    if std::env::var("UPDATE_CONTRACT_FIXTURES").as_deref() != Ok("1") {
        return;
    }
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/contract");
    std::fs::create_dir_all(&dir).unwrap();
    let mut text = serde_json::to_string_pretty(v).unwrap();
    text.push('\n');
    std::fs::write(dir.join(name), text).unwrap();
}

#[tokio::test]
async fn write_contract_fixtures_from_a_full_mode_a_flow() {
    let state = app_state();
    let clients = fake_clients(vec![Ok(brief_json())]);
    cmd::set_api_key(&state, KEY).unwrap();
    set_reviewer(&state, "Jane Architect");

    write_fixture("settings.json", &cmd::get_settings(&state).unwrap());
    write_fixture("criteria.json", &cmd::get_criteria(&state).unwrap());
    write_fixture("data_notice.json", &cmd::data_notice(&state).unwrap());
    write_fixture("health.json", &cmd::health_check(&state, &clients).await.unwrap());
    write_fixture(
        "local_model_status.json",
        &cmd::local_model_status(&state, &clients).await.unwrap(),
    );
    cmd::ack_data_notice(&state).unwrap();

    let view = cmd::start_describe(&state, &clients, DESCRIPTION).await.unwrap();
    assert_eq!(view.session.stage, "review");
    write_fixture("session_view_review.json", &view);
    let id = view.session.id;
    cmd::confirm_brief(&state, &id).unwrap();

    let sink = RecordingSink::new();
    let done = cmd::run_decisions(&state, &clients, &id, &sink).await.unwrap();
    assert!(done.report.is_some());
    write_fixture("session_view_done.json", &done);
    let event = sink
        .events()
        .into_iter()
        .find(|p| p.message.is_some())
        .expect("a decisions progress event with a message");
    write_fixture("progress_event.json", &event);
    write_fixture("session_summaries.json", &cmd::list_sessions(&state).unwrap());

    let decision = &done.report.unwrap().decisions[0];
    let other = decision.options.iter().find(|o| o.id != decision.decision.choice).unwrap();
    cmd::review(&state, &decision.decision.id, ReviewAction::Accept, None, None).unwrap();
    let reviewed = cmd::review(
        &state,
        &decision.decision.id,
        ReviewAction::Override,
        Some(other.id.clone()),
        Some("The client already runs it".to_string()),
    )
    .unwrap();
    write_fixture("decision_view_reviewed.json", &reviewed);

    // A staged error: the local model returns malformed JSON twice.
    let failing = fake_clients(vec![Ok("not json".to_string()), Ok("still not json".to_string())]);
    let err: AppError = cmd::start_describe(&state, &failing, DESCRIPTION).await.unwrap_err();
    assert_eq!(err.code, "brief_extraction_failed");
    assert_eq!(err.stage.as_deref(), Some("brief"));
    write_fixture("app_error.json", &err);

    // An unstaged error omits `stage` entirely (`stage?: string`).
    let unstaged = to_json(&cmd::set_api_key(&state, "").unwrap_err());
    assert!(unstaged.get("stage").is_none());
}
