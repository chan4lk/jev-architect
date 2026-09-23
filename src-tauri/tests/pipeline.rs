//! Integration tests for `bistec_architect::pipeline`, with `FakeJev`,
//! `FakeLocalModel`, an in-memory `Store` and the bundled catalogue.
//! Covers AC-6 (routing), AC-7 (core), AC-9, AC-12 (precedent), AC-16, AC-17
//! (pipeline part), and resume-from-failed-stage.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use bistec_architect::docs::parse_document;
use bistec_architect::jev::{
    Answer, DecisionClient, DecisionRequest, DecisionResponse, FakeJev, JevError, Question, Usage,
};
use bistec_architect::model::brief::{Budget, BriefItem, Compliance, Scale, Timeline};
use bistec_architect::model::catalog::{Catalog, Ring};
use bistec_architect::model::decision::{ReasonCode, Route};
use bistec_architect::model::review::{NewReview, ReviewAction};
use bistec_architect::ollama::FakeLocalModel;
use bistec_architect::pipeline::{
    build_report, confirm_brief, list_sessions, run_decisions, session_view, start_describe, start_upload,
    update_brief, Deps, NoopSink, RecordingSink,
};
use bistec_architect::store::{SessionRow, Store};
use chrono::Utc;
use serde_json::json;

// ---- a scripted Jev that answers any request ---------------------------

/// How the fake answers: Nouls by first matching key prefix (default 0.9),
/// Choices by exact key (default: the first option), Scores 4 for the
/// option the same request's Choice picks and 1 otherwise (2 if the Choice
/// isn't in the request).
#[derive(Clone)]
struct Script {
    nouls: Vec<(String, f64)>,
    choices: Vec<(String, String)>,
    /// Fail the first request that has a key with this prefix.
    fail_once_on: Option<String>,
}

impl Script {
    fn new() -> Self {
        Script {
            nouls: vec![
                ("gate__injection".to_string(), 0.0),
                ("context__compliance__".to_string(), 0.0),
            ],
            choices: vec![],
            fail_once_on: None,
        }
    }

    fn noul(mut self, prefix: &str, v: f64) -> Self {
        self.nouls.insert(0, (prefix.to_string(), v));
        self
    }

    fn choice(mut self, key: &str, option: &str) -> Self {
        self.choices.push((key.to_string(), option.to_string()));
        self
    }

    fn fail_once_on(mut self, prefix: &str) -> Self {
        self.fail_once_on = Some(prefix.to_string());
        self
    }

    fn chosen(&self, key: &str, criteria: &BTreeMap<String, Option<serde_json::Value>>) -> String {
        self.choices
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
            .unwrap_or_else(|| criteria.keys().next().expect("choice has options").clone())
    }

    fn answer(&self, req: &DecisionRequest) -> DecisionResponse {
        let mut answers = BTreeMap::new();
        for (key, q) in &req.questions {
            let a = match q {
                Question::Noul { .. } => Answer::Noul {
                    noul: self
                        .nouls
                        .iter()
                        .find(|(p, _)| key.starts_with(p.as_str()))
                        .map(|(_, v)| *v)
                        .unwrap_or(0.9),
                },
                Question::Choice { criteria, .. } => {
                    let chosen = self.chosen(key, criteria);
                    let others = (criteria.len().max(2) - 1) as f64;
                    let probabilities = criteria
                        .keys()
                        .map(|k| (k.clone(), if *k == chosen { 0.8 } else { 0.2 / others }))
                        .collect();
                    Answer::Choice {
                        choice: chosen,
                        confidence: 0.8,
                        probabilities,
                    }
                }
                Question::Score { .. } => {
                    let parts: Vec<&str> = key.split("__").collect();
                    let (type_id, option_id) = (parts[1], parts[2]);
                    let choice_key = format!("choice__{type_id}");
                    let score = match req.questions.get(&choice_key) {
                        Some(Question::Choice { criteria, .. }) => {
                            if self.chosen(&choice_key, criteria) == option_id {
                                4.0
                            } else {
                                1.0
                            }
                        }
                        _ => 2.0,
                    };
                    Answer::Score {
                        score,
                        confidence: 0.9,
                        probabilities: BTreeMap::new(),
                        legend: None,
                    }
                }
            };
            answers.insert(key.clone(), a);
        }
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
}

type Handler = Box<dyn Fn(&DecisionRequest) -> Result<DecisionResponse, JevError> + Send + Sync>;

fn fake_jev(script: Script) -> Arc<FakeJev<Handler>> {
    let failed = AtomicBool::new(false);
    let handler: Handler = Box::new(move |req| {
        if let Some(prefix) = &script.fail_once_on {
            if req.questions.keys().any(|k| k.starts_with(prefix.as_str())) && !failed.swap(true, Ordering::SeqCst) {
                return Err(JevError::Http {
                    status: 500,
                    body: "scripted failure".to_string(),
                });
            }
        }
        Ok(script.answer(req))
    });
    Arc::new(FakeJev::new(handler))
}

fn requests_with(fake: &FakeJev<Handler>, prefix: &str) -> Vec<DecisionRequest> {
    fake.requests()
        .into_iter()
        .filter(|r| r.questions.keys().any(|k| k.starts_with(prefix)))
        .collect()
}

// ---- deps + fixtures ----------------------------------------------------

fn mode_a_brief_json() -> String {
    json!({
        "summary": "A customer self-service portal for an insurer.",
        "context": {
            "scale": "medium", "budget": "moderate", "timeline": "normal",
            "team_size": "small", "compliance": ["gdpr"], "data_sensitivity": "confidential"
        },
        "requirements": [{"text": "Policyholders track claims", "sources": ["\"track claims\""]}],
        "nfrs": [{"text": "Pages load in under two seconds", "sources": []}],
        "constraints": [],
        "team_skills": [{"text": "The team knows .NET", "sources": []}],
        "mentioned_technologies": [".NET", "Azure"]
    })
    .to_string()
}

fn deps_with(fake: &Arc<FakeJev<Handler>>, local: FakeLocalModel) -> (Deps, Arc<FakeLocalModel>) {
    let local = Arc::new(local);
    let jev: Arc<dyn DecisionClient> = fake.clone();
    let deps = Deps {
        catalog: Arc::new(Catalog::bundled().unwrap()),
        store: Arc::new(Store::open_in_memory().unwrap()),
        jev,
        local: local.clone(),
    };
    (deps, local)
}

fn mode_a_deps(fake: &Arc<FakeJev<Handler>>) -> Deps {
    deps_with(fake, FakeLocalModel::new("m", true, vec![Ok(mode_a_brief_json())])).0
}

const DESCRIPTION: &str = "We need a customer self-service portal for an insurer, built by a .NET team on Azure.";

/// Mode A session, confirmed, notice acknowledged.
async fn ready_session(d: &Deps) -> String {
    let id = start_describe(d, DESCRIPTION).await.unwrap();
    confirm_brief(d, &id).unwrap();
    d.store.ack_data_notice().unwrap();
    id
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/docs").join(name)
}

// ---- Mode A end to end (AC-7) --------------------------------------------

#[tokio::test]
async fn mode_a_edited_brief_is_what_jev_sees_and_the_report_has_applicable_types_only() {
    let fake = fake_jev(Script::new().noul("applies__css", 0.1));
    let d = mode_a_deps(&fake);

    let id = start_describe(&d, DESCRIPTION).await.unwrap();
    let view = session_view(&d, &id).unwrap();
    assert_eq!(view.session.stage, "review");
    assert_eq!(view.session.mode, "describe");
    assert!(view.doc_path.is_none());
    let mut brief = view.brief.unwrap();
    assert!(brief.requirements[0].sources.is_empty(), "Mode A sources are cleared");

    confirm_brief(&d, &id).unwrap();
    d.store.ack_data_notice().unwrap();

    // Editing a confirmed brief requires confirming it again.
    brief.summary = "EDITED: an event-driven claims portal".to_string();
    brief.constraints.push(BriefItem {
        text: "Must stay under $3k a month".to_string(),
        sources: vec![],
    });
    update_brief(&d, &id, &brief).unwrap();
    assert!(session_view(&d, &id).unwrap().session.brief_confirmed_at.is_none());
    let err = run_decisions(&d, &id, &NoopSink).await.unwrap_err();
    assert_eq!(err.code(), "validation");
    assert_eq!(fake.call_count(), 0);

    confirm_brief(&d, &id).unwrap();
    let sink = RecordingSink::new();
    run_decisions(&d, &id, &sink).await.unwrap();

    // Every Jev request's state carries the edited brief.
    for req in fake.requests() {
        assert_eq!(req.state["brief"]["summary"], "EDITED: an event-driven claims portal");
        assert!(req.state.to_string().contains("Must stay under $3k a month"));
    }

    let view = session_view(&d, &id).unwrap();
    assert_eq!(view.session.stage, "done");
    let report = view.report.expect("report after done");
    let type_ids: Vec<&str> = report.decisions.iter().map(|v| v.decision.type_id.as_str()).collect();
    assert_eq!(&type_ids[..2], &["cloud-platform", "backend-platform"]);
    assert!(!type_ids.contains(&"css"));
    assert_eq!(report.not_applicable.len(), 1);
    assert_eq!(report.not_applicable[0].type_id, "css");
    assert_eq!(report.not_applicable[0].type_name, "CSS");
    assert!(report.decisions.len() > 10);
    for v in &report.decisions {
        assert_eq!(v.decision.model_snapshot, "typesafe/jev-1.13-20260901");
        assert_eq!(v.decision.request_hash.len(), 64);
        assert!(v.decision.cited_sections.is_empty());
        assert_eq!(v.status_label, "Proposed (AI) — pending approval");
        assert!(!v.options.is_empty());
    }
    // Every call's usage was logged.
    assert_eq!(report.input_tokens, 100 * fake.call_count() as u64);
    assert!(report.cost_usd.is_some());
    assert_eq!(report.criteria.len(), 6);
    assert!(!report.truncated);

    // Progress was reported through to done, with k/n for decisions.
    let events = sink.events();
    assert!(events.iter().any(|p| p.stage == "decisions" && p.total > 0 && p.done == p.total));
    assert_eq!(events.last().unwrap().stage, "done");
}

#[tokio::test]
async fn pipeline_futures_are_send() {
    fn assert_send<T: Send>(_: &T) {}
    let fake = fake_jev(Script::new());
    let d = mode_a_deps(&fake);
    let path = fixture("sample.md");
    assert_send(&start_upload(&d, &path, &NoopSink));
    let describe = start_describe(&d, DESCRIPTION);
    assert_send(&describe);
    let id = describe.await.unwrap();
    confirm_brief(&d, &id).unwrap();
    d.store.ack_data_notice().unwrap();
    let fut = run_decisions(&d, &id, &NoopSink);
    assert_send(&fut);
    fut.await.unwrap();
}

#[tokio::test]
async fn describe_rejects_tiny_input() {
    let fake = fake_jev(Script::new());
    let d = mode_a_deps(&fake);
    let err = start_describe(&d, "too short").await.unwrap_err();
    assert_eq!(err.code(), "validation");
    assert!(list_sessions(&d).is_empty());
}

#[tokio::test]
async fn failed_mode_a_extraction_marks_the_session_failed_brief() {
    let fake = fake_jev(Script::new());
    let (d, _) = deps_with(
        &fake,
        FakeLocalModel::new("m", true, vec![Ok("nope".to_string()), Ok("still nope".to_string())]),
    );
    let err = start_describe(&d, DESCRIPTION).await.unwrap_err();
    assert_eq!(err.code(), "brief_extraction_failed");
    assert_eq!(err.stage(), Some("brief"));
    let sessions = list_sessions(&d);
    assert_eq!(sessions[0].stage, "failed:brief");
}

// ---- preconditions (FR-8, AC-17) ----------------------------------------

#[tokio::test]
async fn data_notice_not_acked_blocks_every_jev_call() {
    let fake = fake_jev(Script::new());
    let d = mode_a_deps(&fake);
    let id = start_describe(&d, DESCRIPTION).await.unwrap();
    confirm_brief(&d, &id).unwrap();

    let err = run_decisions(&d, &id, &NoopSink).await.unwrap_err();
    assert_eq!(err.code(), "data_notice_required");
    assert_eq!(fake.call_count(), 0);
}

#[tokio::test]
async fn unconfirmed_brief_blocks_the_decision_pass() {
    let fake = fake_jev(Script::new());
    let d = mode_a_deps(&fake);
    let id = start_describe(&d, DESCRIPTION).await.unwrap();
    d.store.ack_data_notice().unwrap();

    let err = run_decisions(&d, &id, &NoopSink).await.unwrap_err();
    assert_eq!(err.code(), "validation");
    assert_eq!(fake.call_count(), 0);
    assert_eq!(run_decisions(&d, "nope", &NoopSink).await.unwrap_err().code(), "not_found");
}

// ---- gates (AC-9) --------------------------------------------------------

#[tokio::test]
async fn out_of_remit_stops_before_applicability_and_decisions() {
    let fake = fake_jev(Script::new().noul("gate__is_technical_request", 0.2));
    let d = mode_a_deps(&fake);
    let id = ready_session(&d).await;

    run_decisions(&d, &id, &NoopSink).await.unwrap();
    assert_eq!(session_view(&d, &id).unwrap().session.stage, "out_of_remit");
    assert_eq!(fake.call_count(), 1);
    assert!(requests_with(&fake, "applies__").is_empty());
    assert!(requests_with(&fake, "choice__").is_empty());

    let report = build_report(&d, &id).unwrap().expect("out-of-remit report");
    assert!(report.decisions.is_empty());
    assert_eq!(report.gates.is_technical_request, 0.2);

    // Running again stays out of remit and calls nothing.
    run_decisions(&d, &id, &NoopSink).await.unwrap();
    assert_eq!(fake.call_count(), 1);
}

#[tokio::test]
async fn injection_routes_every_decision_to_the_architect() {
    let fake = fake_jev(Script::new().noul("gate__injection", 0.4));
    let d = mode_a_deps(&fake);
    let id = ready_session(&d).await;

    run_decisions(&d, &id, &NoopSink).await.unwrap();
    let report = build_report(&d, &id).unwrap().unwrap();
    assert!(!report.decisions.is_empty());
    for v in &report.decisions {
        assert_eq!(v.decision.route, Route::NeedsArchitect, "{}", v.decision.type_id);
        assert!(v.decision.reasons.contains(&ReasonCode::PossibleInjection));
    }
}

// ---- rules (AC-8, pipeline part) -------------------------------------------

#[tokio::test]
async fn dotnet_backend_never_asks_about_node_rows() {
    let fake = fake_jev(Script::new().choice("choice__backend-platform", "dotnet"));
    let d = mode_a_deps(&fake);
    let id = ready_session(&d).await;
    run_decisions(&d, &id, &NoopSink).await.unwrap();

    let applicability = requests_with(&fake, "applies__");
    assert_eq!(applicability.len(), 1);
    let keys: Vec<&String> = applicability[0].questions.keys().collect();
    assert!(keys.iter().any(|k| *k == "applies__rest-api-dotnet"));
    assert!(keys.iter().any(|k| *k == "applies__testing-dotnet"));
    for node_row in ["rest-api-node", "testing-node"] {
        assert!(!keys.iter().any(|k| k.contains(node_row)));
        assert!(requests_with(&fake, &format!("choice__{node_row}")).is_empty());
    }
}

/// Baseline types (IaC, CI/CD, monitoring — required on every BISTEC project)
/// are never put to the applicability question and are decided even when Jev
/// would have said "not applicable" for everything.
#[tokio::test]
async fn baseline_types_skip_applicability_and_are_always_decided() {
    let fake = fake_jev(
        Script::new()
            .choice("choice__cloud-platform", "azure")
            .noul("applies__", 0.05),
    );
    let d = mode_a_deps(&fake);
    let id = ready_session(&d).await;
    run_decisions(&d, &id, &NoopSink).await.unwrap();

    let applicability = requests_with(&fake, "applies__");
    let keys: Vec<&String> = applicability.iter().flat_map(|r| r.questions.keys()).collect();
    for baseline in ["iac-azure", "ci-cd", "monitoring"] {
        assert!(!keys.iter().any(|k| **k == format!("applies__{baseline}")), "{baseline} was asked");
        assert_eq!(requests_with(&fake, &format!("choice__{baseline}")).len(), 1, "{baseline} not decided");
    }
    // A non-baseline type Jev judged not applicable is still pruned.
    assert!(requests_with(&fake, "choice__document-db").is_empty());
}

// ---- precedent (AC-12) ---------------------------------------------------

#[tokio::test]
async fn precedent_state_carries_the_three_most_recent_accepted_decisions() {
    let fake = fake_jev(Script::new());
    let d = mode_a_deps(&fake);

    for i in 0..5 {
        let session_id = format!("past-{i}");
        d.store
            .create_session(&SessionRow {
                id: session_id.clone(),
                created_at: Utc::now(),
                title: format!("Past {i}"),
                mode: "describe".to_string(),
                input_text: None,
                doc_name: None,
                sections_json: None,
                brief_json: Some(json!({ "summary": format!("Past project {i}") }).to_string()),
                brief_confirmed_at: None,
                stage: "done".to_string(),
                error: None,
            })
            .unwrap();
        d.store
            .insert_decision(&bistec_architect::model::decision::DecisionResult {
                id: format!("past-decision-{i}"),
                session_id,
                type_id: "document-db".to_string(),
                choice: "cosmos-db".to_string(),
                confidence: 0.9,
                probabilities: BTreeMap::from([("cosmos-db".to_string(), 0.9)]),
                option_scores: vec![],
                route: Route::Proposed,
                reasons: vec![],
                model_snapshot: "m".to_string(),
                request_hash: "h".to_string(),
                cited_sections: vec![],
            })
            .unwrap();
        d.store
            .append_review(&NewReview {
                decision_id: format!("past-decision-{i}"),
                action: ReviewAction::Accept,
                option_id: None,
                reviewer: "Architect".to_string(),
                reason: None,
            })
            .unwrap();
    }

    let id = ready_session(&d).await;
    run_decisions(&d, &id, &NoopSink).await.unwrap();

    let doc_db = requests_with(&fake, "choice__document-db");
    assert_eq!(doc_db.len(), 1);
    let precedent = doc_db[0].state["bistec_precedent"].as_array().expect("precedent in state");
    assert_eq!(precedent.len(), 3);
    let summaries: Vec<&str> = precedent.iter().map(|p| p["summary"].as_str().unwrap()).collect();
    assert_eq!(summaries, vec!["Past project 4", "Past project 3", "Past project 2"]);
    assert_eq!(precedent[0]["option_id"], "cosmos-db");
    assert_eq!(doc_db[0].state["bistec_standard"]["type"], "Document DB");

    // Types without accepted history carry no precedent.
    let css = requests_with(&fake, "choice__css");
    assert!(css[0].state.get("bistec_precedent").is_none());
}

// ---- Mode B routing (AC-6) ----------------------------------------------

#[tokio::test]
async fn small_document_takes_the_small_path_with_jev_context_choices() {
    let fake = fake_jev(
        Script::new()
            .choice("context__budget", "tight")
            .choice("context__timeline", "not_stated")
            .noul("context__compliance__gdpr", 0.9),
    );
    let (d, local) = deps_with(&fake, FakeLocalModel::new("m", true, vec![]));

    // The small path calls Jev, so the notice must be acknowledged first.
    let err = start_upload(&d, &fixture("sample.md"), &NoopSink).await.unwrap_err();
    assert_eq!(err.code(), "data_notice_required");
    assert_eq!(fake.call_count(), 0);
    assert!(list_sessions(&d).is_empty());

    d.store.ack_data_notice().unwrap();
    let id = start_upload(&d, &fixture("sample.md"), &NoopSink).await.unwrap();

    assert_eq!(fake.call_count(), 1);
    assert!(local.calls().is_empty(), "the small path never uses the local model");
    let req = &fake.requests()[0];
    assert!(req.questions.keys().all(|k| k.starts_with("context__")));
    let choices: Vec<_> = req
        .questions
        .values()
        .filter_map(|q| match q {
            Question::Choice { criteria, .. } => Some(criteria),
            _ => None,
        })
        .collect();
    assert_eq!(choices.len(), 5);
    assert!(choices.iter().all(|c| c.contains_key("not_stated")));
    assert!(req.state["document_sections"].as_array().unwrap().len() > 1);

    let view = session_view(&d, &id).unwrap();
    assert_eq!(view.doc_path.as_deref(), Some("small"));
    assert_eq!(view.session.stage, "review");
    assert_eq!(view.session.mode, "upload");
    assert_eq!(view.session.doc_name.as_deref(), Some("sample.md"));
    let brief = view.brief.unwrap();
    assert_eq!(brief.context.budget, Budget::Tight);
    assert_eq!(brief.context.timeline, Timeline::Unknown);
    assert_eq!(brief.context.compliance, vec![Compliance::Gdpr]);
    assert!(brief.mentioned_technologies.contains(&"Microsoft Azure".to_string()));
    assert!(!brief.summary.is_empty());
    assert!(view.sections.unwrap().len() > 1);

    // Decisions on the small path cite the sections that were sent.
    confirm_brief(&d, &id).unwrap();
    run_decisions(&d, &id, &NoopSink).await.unwrap();
    let report = build_report(&d, &id).unwrap().unwrap();
    assert!(report.decisions.iter().all(|v| v.decision.cited_sections.contains(&"S1".to_string())));
}

#[tokio::test]
async fn large_document_takes_the_large_path_with_section_tagged_merge() {
    let path = fixture("large.md");
    let sections = parse_document(&path).unwrap();
    let n = sections.len();
    assert!(n > 2);

    let responses = (0..n)
        .map(|i| {
            Ok(json!({
                "summary": if i == 0 { "" } else { "Insurer self-service portal." },
                "context": {
                    "scale": if i == 1 { "medium" } else { "unknown" },
                    "budget": "unknown", "timeline": "unknown", "team_size": "unknown",
                    "compliance": [], "data_sensitivity": "unknown"
                },
                "requirements": [
                    {"text": "Common requirement", "sources": ["a quote the model invented"]},
                    {"text": format!("Requirement from section {i}"), "sources": []}
                ],
                "nfrs": [], "constraints": [], "team_skills": [],
                "mentioned_technologies": ["Entra ID"]
            })
            .to_string())
        })
        .collect();
    let fake = fake_jev(Script::new());
    let (d, local) = deps_with(&fake, FakeLocalModel::new("m", true, responses));

    let sink = RecordingSink::new();
    let id = start_upload(&d, &path, &sink).await.unwrap();

    assert_eq!(fake.call_count(), 0, "the large path never calls Jev");
    assert_eq!(local.calls().len(), n);
    let brief_events: Vec<_> = sink.events().into_iter().filter(|p| p.stage == "brief").collect();
    assert_eq!(brief_events.len(), n);
    assert_eq!(brief_events.last().unwrap().done, n);

    let view = session_view(&d, &id).unwrap();
    assert_eq!(view.doc_path.as_deref(), Some("large"));
    assert_eq!(view.session.stage, "review");
    let brief = view.brief.unwrap();
    let ids: Vec<String> = sections.iter().map(|s| s.id.clone()).collect();
    assert_eq!(brief.requirements.len(), 1 + n);
    for item in &brief.requirements {
        assert!(!item.sources.is_empty());
        assert!(item.sources.iter().all(|s| ids.contains(s)), "{:?}", item.sources);
    }
    assert_eq!(brief.requirements[0].sources, ids);
    assert_eq!(brief.context.scale, Scale::Medium);
    assert_eq!(brief.summary, "Insurer self-service portal.");
    assert!(brief.mentioned_technologies.contains(&"Entra ID".to_string()));

    // Decisions on the large path cite the brief's sources; the document is
    // over budget, so the evidence is truncated.
    confirm_brief(&d, &id).unwrap();
    d.store.ack_data_notice().unwrap();
    run_decisions(&d, &id, &NoopSink).await.unwrap();
    let report = build_report(&d, &id).unwrap().unwrap();
    assert!(report.truncated);
    assert!(report.decisions.iter().all(|v| v.decision.cited_sections == ids));
    for req in fake.requests() {
        let tokens = serde_json::to_string(&req.state).unwrap().chars().count().div_ceil(4);
        assert!(tokens <= 20_000, "state of {tokens} tokens exceeds the budget");
    }
}

// ---- resume + replay ------------------------------------------------------

#[tokio::test]
async fn a_failed_stage_resumes_without_re_asking_earlier_stages() {
    let fake = fake_jev(Script::new().fail_once_on("applies__"));
    let d = mode_a_deps(&fake);
    let id = ready_session(&d).await;

    let err = run_decisions(&d, &id, &NoopSink).await.unwrap_err();
    assert_eq!(err.code(), "jev");
    assert_eq!(err.stage(), Some("applicability"));
    let view = session_view(&d, &id).unwrap();
    assert_eq!(view.session.stage, "failed:applicability");
    assert!(view.session.error.is_some());
    assert!(view.report.is_none());

    run_decisions(&d, &id, &NoopSink).await.unwrap();
    assert_eq!(requests_with(&fake, "gate__").len(), 1);
    assert_eq!(requests_with(&fake, "platform__").len(), 1);
    assert_eq!(requests_with(&fake, "applies__").len(), 2);
    let view = session_view(&d, &id).unwrap();
    assert_eq!(view.session.stage, "done");
    assert!(view.session.error.is_none());
}

#[tokio::test]
async fn reopening_a_session_replays_the_report_with_zero_calls() {
    let fake = fake_jev(Script::new());
    let d = mode_a_deps(&fake);
    let id = ready_session(&d).await;
    run_decisions(&d, &id, &NoopSink).await.unwrap();

    let calls = fake.call_count();
    let first = build_report(&d, &id).unwrap().unwrap();
    let second = build_report(&d, &id).unwrap().unwrap();
    let view = session_view(&d, &id).unwrap();
    assert_eq!(fake.call_count(), calls, "replay must not call Jev");
    assert_eq!(first, second);
    assert_eq!(view.report.unwrap(), first);
    assert_eq!(serde_json::to_string(&first).unwrap(), serde_json::to_string(&second).unwrap());

    // run_decisions on a finished session is a no-op too.
    run_decisions(&d, &id, &NoopSink).await.unwrap();
    assert_eq!(fake.call_count(), calls);

    let summaries = list_sessions(&d);
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].decision_count, first.decisions.len());
    assert_eq!(summaries[0].unreviewed_count, first.decisions.len());
    let needs_architect = first.decisions.iter().filter(|v| v.decision.route == Route::NeedsArchitect).count();
    assert_eq!(summaries[0].needs_architect_count, needs_architect);
}

#[tokio::test]
async fn editing_the_brief_after_decisions_clears_them_for_a_fresh_run() {
    let fake = fake_jev(Script::new());
    let d = mode_a_deps(&fake);
    let id = ready_session(&d).await;
    run_decisions(&d, &id, &NoopSink).await.unwrap();

    let brief = session_view(&d, &id).unwrap().brief.unwrap();
    update_brief(&d, &id, &brief).unwrap();
    let view = session_view(&d, &id).unwrap();
    assert_eq!(view.session.stage, "review");
    assert!(view.report.is_none());
    assert_eq!(list_sessions(&d)[0].decision_count, 0);

    confirm_brief(&d, &id).unwrap();
    let before = requests_with(&fake, "gate__").len();
    run_decisions(&d, &id, &NoopSink).await.unwrap();
    assert_eq!(requests_with(&fake, "gate__").len(), before + 1);
    let report = build_report(&d, &id).unwrap().unwrap();
    let mut types: Vec<&str> = report.decisions.iter().map(|v| v.decision.type_id.as_str()).collect();
    let total = types.len();
    types.dedup();
    assert_eq!(types.len(), total, "stale decisions from the first run are not reported");
}

// ---- edge case: every option of an applicable type is on hold -----------

/// A clone of the bundled catalogue with every option of `type_id` forced to
/// `Ring::Hold` (spec edge case "every option for a type is on hold").
fn catalog_with_every_option_on_hold(type_id: &str) -> Catalog {
    let mut cat = Catalog::bundled().unwrap();
    let dt = cat.types.iter_mut().find(|t| t.id == type_id).expect("type exists in the bundled catalogue");
    for opt in dt.options.iter_mut() {
        opt.ring = Ring::Hold;
    }
    cat
}

#[tokio::test]
async fn every_option_on_hold_is_skipped_with_no_eligible_options_and_never_asked() {
    let fake = fake_jev(Script::new());
    let local = Arc::new(FakeLocalModel::new("m", true, vec![Ok(mode_a_brief_json())]));
    let jev: Arc<dyn DecisionClient> = fake.clone();
    let d = Deps {
        catalog: Arc::new(catalog_with_every_option_on_hold("document-db")),
        store: Arc::new(Store::open_in_memory().unwrap()),
        jev,
        local,
    };
    let id = ready_session(&d).await;
    run_decisions(&d, &id, &NoopSink).await.unwrap();

    // No request was ever sent for the all-hold type (DESCRIPTION never
    // mentions any of its options, so none becomes eligible).
    assert!(requests_with(&fake, "choice__document-db").is_empty());
    assert!(requests_with(&fake, "score__document-db").is_empty());

    let report = build_report(&d, &id).unwrap().unwrap();
    assert!(!report.decisions.iter().any(|v| v.decision.type_id == "document-db"));
    // It is applicable (not in `not_applicable`) but skipped for lack of
    // eligible options, not because it doesn't apply to this project.
    assert!(!report.not_applicable.iter().any(|na| na.type_id == "document-db"));

    let decisions_stage = d.store.get_stage_result(&id, "decisions").expect("decisions stage result");
    let skipped = decisions_stage["skipped"].as_array().expect("skipped array");
    let entry = skipped
        .iter()
        .find(|s| s["type_id"] == "document-db")
        .expect("document-db recorded as skipped");
    assert_eq!(entry["reason"], "no_eligible_options");
}
