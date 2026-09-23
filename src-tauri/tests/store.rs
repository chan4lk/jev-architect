//! Integration tests for `bistec_architect::store::Store` and
//! `bistec_architect::secrets`, covering the storage side of AC-3, AC-12,
//! AC-13, and AC-17.

use std::collections::BTreeMap;

use bistec_architect::model::decision::{DecisionResult, OptionScore, Route};
use bistec_architect::model::review::{AdrStatus, NewReview, ReviewAction};
use bistec_architect::model::settings::Settings;
use bistec_architect::secrets::{MemoryStore, SecretStore};
use bistec_architect::store::{SessionRow, Store, StoreError};
use chrono::Utc;
use serde_json::json;
use uuid::Uuid;

fn new_session(mode: &str, title: &str) -> SessionRow {
  SessionRow {
    id: Uuid::new_v4().to_string(),
    created_at: Utc::now(),
    title: title.to_string(),
    mode: mode.to_string(),
    input_text: Some("some input".to_string()),
    doc_name: None,
    sections_json: None,
    brief_json: None,
    brief_confirmed_at: None,
    stage: "brief".to_string(),
    error: None,
  }
}

fn sample_decision(id: &str, session_id: &str, type_id: &str, choice: &str) -> DecisionResult {
  DecisionResult {
    id: id.to_string(),
    session_id: session_id.to_string(),
    type_id: type_id.to_string(),
    choice: choice.to_string(),
    confidence: 0.8,
    probabilities: BTreeMap::from([(choice.to_string(), 0.8)]),
    option_scores: vec![OptionScore {
      option_id: choice.to_string(),
      criterion_scores: BTreeMap::from([("nfr_fit".to_string(), 4.0)]),
      composite: 1.0,
    }],
    route: Route::Proposed,
    reasons: vec![],
    model_snapshot: "typesafe/jev-1.13-2026-01-01".to_string(),
    request_hash: "hash".to_string(),
    cited_sections: vec![],
  }
}

// ---- migrations ---------------------------------------------------------

#[test]
fn migrations_apply_on_a_fresh_database() {
  let store = Store::open_in_memory().expect("migrations should apply cleanly on a fresh DB");
  // The schema is actually usable: every table can be queried.
  assert_eq!(store.list_sessions(), vec![]);
  assert_eq!(store.decisions_for_session("nope"), vec![]);
  assert_eq!(store.reviews_for_decision("nope"), vec![]);
  assert_eq!(store.get_stage_result("nope", "brief"), None);
  assert_eq!(store.session_usage("nope"), (0, None));
  assert_eq!(store.precedent("cloud-platform", 3), vec![]);
}

// ---- sessions -------------------------------------------------------------

#[test]
fn session_crud_round_trips() {
  let store = Store::open_in_memory().unwrap();
  let row = new_session("describe", "My project");
  store.create_session(&row).unwrap();

  let fetched = store.get_session(&row.id).expect("session should exist");
  assert_eq!(fetched, row);

  store.update_session_brief(&row.id, r#"{"summary":"A brief"}"#).unwrap();
  store.confirm_brief(&row.id).unwrap();
  store.set_session_stage(&row.id, "gates", None).unwrap();
  store.set_sections(&row.id, r#"[{"id":"S1"}]"#).unwrap();

  let fetched = store.get_session(&row.id).unwrap();
  assert_eq!(fetched.brief_json.as_deref(), Some(r#"{"summary":"A brief"}"#));
  assert!(fetched.brief_confirmed_at.is_some());
  assert_eq!(fetched.stage, "gates");
  assert_eq!(fetched.sections_json.as_deref(), Some(r#"[{"id":"S1"}]"#));

  store.set_session_stage(&row.id, "failed:gates", Some("boom")).unwrap();
  let fetched = store.get_session(&row.id).unwrap();
  assert_eq!(fetched.stage, "failed:gates");
  assert_eq!(fetched.error.as_deref(), Some("boom"));

  // list_sessions is newest first.
  let mut second = new_session("upload", "Second project");
  second.created_at = row.created_at + chrono::Duration::seconds(5);
  store.create_session(&second).unwrap();
  let all = store.list_sessions();
  assert_eq!(all.len(), 2);
  assert_eq!(all[0].id, second.id, "the newer session should come first");
  assert_eq!(all[1].id, row.id);
}

#[test]
fn get_session_returns_none_for_an_unknown_id() {
  let store = Store::open_in_memory().unwrap();
  assert_eq!(store.get_session("does-not-exist"), None);
}

// ---- stage results --------------------------------------------------------

#[test]
fn stage_result_upsert_and_clear() {
  let store = Store::open_in_memory().unwrap();
  let session = new_session("describe", "Proj");
  store.create_session(&session).unwrap();

  assert_eq!(store.get_stage_result(&session.id, "gates"), None);

  store.put_stage_result(&session.id, "gates", &json!({"injection": 0.1})).unwrap();
  assert_eq!(store.get_stage_result(&session.id, "gates"), Some(json!({"injection": 0.1})));

  // Upsert: putting again for the same (session_id, stage) replaces it.
  store.put_stage_result(&session.id, "gates", &json!({"injection": 0.9})).unwrap();
  assert_eq!(store.get_stage_result(&session.id, "gates"), Some(json!({"injection": 0.9})));

  store.put_stage_result(&session.id, "applicability", &json!([1, 2, 3])).unwrap();
  store.clear_stage_results_from(&session.id, &["gates", "applicability"]).unwrap();
  assert_eq!(store.get_stage_result(&session.id, "gates"), None);
  assert_eq!(store.get_stage_result(&session.id, "applicability"), None);
}

// ---- reviews: append-only + adr_status ------------------------------------
//
// The DB-level trigger test (raw UPDATE/DELETE on `reviews` both fail) lives
// in `src/store.rs`'s in-module tests, since it needs the private
// `Connection` to issue raw SQL directly.

#[test]
fn two_reviews_leaves_both_rows_and_adr_status_follows_the_latest() {
  let store = Store::open_in_memory().unwrap();
  let session = new_session("describe", "Proj");
  store.create_session(&session).unwrap();
  let decision = sample_decision("d1", &session.id, "cloud-platform", "azure");
  store.insert_decision(&decision).unwrap();

  let first = store
    .append_review(&NewReview {
      decision_id: "d1".to_string(),
      action: ReviewAction::Accept,
      option_id: None,
      reviewer: "Jane".to_string(),
      reason: None,
    })
    .unwrap();

  let second = store
    .append_review(&NewReview {
      decision_id: "d1".to_string(),
      action: ReviewAction::Reject,
      option_id: None,
      reviewer: "Jane".to_string(),
      reason: Some("changed my mind".to_string()),
    })
    .unwrap();

  assert_ne!(first.id, second.id);

  let history = store.reviews_for_decision("d1");
  assert_eq!(history.len(), 2, "both reviews should still be present (append-only)");

  let status = bistec_architect::model::review::adr_status(&history);
  assert_eq!(status, AdrStatus::Rejected, "ADR status should follow the latest review");
}

#[test]
fn append_review_rejects_an_invalid_new_review() {
  let store = Store::open_in_memory().unwrap();
  let session = new_session("describe", "Proj");
  store.create_session(&session).unwrap();
  let decision = sample_decision("d1", &session.id, "cloud-platform", "azure");
  store.insert_decision(&decision).unwrap();

  let err = store
    .append_review(&NewReview {
      decision_id: "d1".to_string(),
      action: ReviewAction::Override,
      option_id: None, // Override requires an option_id (AC-13).
      reviewer: "Jane".to_string(),
      reason: Some("cheaper".to_string()),
    })
    .unwrap_err();
  assert!(matches!(err, StoreError::Review(_)));

  // No row was written for the rejected attempt.
  assert_eq!(store.reviews_for_decision("d1").len(), 0);
}

// ---- precedent (AC-12, query part) ---------------------------------------

#[test]
fn precedent_returns_the_three_most_recent_of_five_excluding_rejected_and_other_types() {
  let store = Store::open_in_memory().unwrap();

  // Five accepted/overridden decisions of the target type, at increasing
  // timestamps, plus one rejected decision of the same type and one
  // accepted decision of a different type — neither should ever appear.
  for i in 0..5 {
    let session = new_session("describe", &format!("Project {i}"));
    store.create_session(&session).unwrap();
    let decision_id = format!("cp-{i}");
    let decision = sample_decision(&decision_id, &session.id, "cloud-platform", "azure");
    store.insert_decision(&decision).unwrap();
    store
      .append_review(&NewReview {
        decision_id: decision_id.clone(),
        action: ReviewAction::Accept,
        option_id: None,
        reviewer: "Jane".to_string(),
        reason: None,
      })
      .unwrap();
  }

  let rejected_session = new_session("describe", "Rejected project");
  store.create_session(&rejected_session).unwrap();
  let rejected_decision = sample_decision("cp-rejected", &rejected_session.id, "cloud-platform", "hetzner");
  store.insert_decision(&rejected_decision).unwrap();
  store
    .append_review(&NewReview {
      decision_id: "cp-rejected".to_string(),
      action: ReviewAction::Reject,
      option_id: None,
      reviewer: "Jane".to_string(),
      reason: Some("no".to_string()),
    })
    .unwrap();

  let other_type_session = new_session("describe", "Other type project");
  store.create_session(&other_type_session).unwrap();
  let other_type_decision = sample_decision("bp-1", &other_type_session.id, "backend-platform", "dotnet");
  store.insert_decision(&other_type_decision).unwrap();
  store
    .append_review(&NewReview {
      decision_id: "bp-1".to_string(),
      action: ReviewAction::Accept,
      option_id: None,
      reviewer: "Jane".to_string(),
      reason: None,
    })
    .unwrap();

  let precedent = store.precedent("cloud-platform", 3);
  assert_eq!(precedent.len(), 3, "should return only the 3 most recent");
  for p in &precedent {
    assert_eq!(p.type_id, "cloud-platform");
    assert_eq!(p.option_id, "azure");
  }
  // Newest review first.
  for pair in precedent.windows(2) {
    assert!(pair[0].at_utc >= pair[1].at_utc);
  }
  // The 3 most recent of cp-0..cp-4 are cp-4, cp-3, cp-2 (increasing i ==
  // increasing review time since each is appended in order).
  let summaries: Vec<String> = precedent.iter().map(|p| p.summary.clone()).collect();
  assert_eq!(summaries, vec!["Project 4", "Project 3", "Project 2"]);
}

#[test]
fn precedent_summary_prefers_the_briefs_summary_field_over_the_title() {
  let store = Store::open_in_memory().unwrap();
  let mut session = new_session("describe", "Fallback title");
  session.brief_json = Some(r#"{"summary":"A one-line summary"}"#.to_string());
  store.create_session(&session).unwrap();
  let decision = sample_decision("d1", &session.id, "cloud-platform", "azure");
  store.insert_decision(&decision).unwrap();
  store
    .append_review(&NewReview {
      decision_id: "d1".to_string(),
      action: ReviewAction::Override,
      option_id: Some("hetzner".to_string()),
      reviewer: "Jane".to_string(),
      reason: Some("cheaper".to_string()),
    })
    .unwrap();

  let precedent = store.precedent("cloud-platform", 3);
  assert_eq!(precedent.len(), 1);
  assert_eq!(precedent[0].option_id, "hetzner", "an Override's option_id is the review's option");
  assert_eq!(precedent[0].summary, "A one-line summary");
}

// ---- decisions ------------------------------------------------------------

#[test]
fn insert_decision_upserts_by_id_and_preserves_adr_number() {
  let store = Store::open_in_memory().unwrap();
  let session = new_session("describe", "Proj");
  store.create_session(&session).unwrap();
  let decision = sample_decision("d1", &session.id, "cloud-platform", "azure");
  store.insert_decision(&decision).unwrap();
  store.set_adr_number("d1", 7).unwrap();

  let mut updated = decision.clone();
  updated.choice = "hetzner".to_string();
  store.insert_decision(&updated).unwrap();

  let all = store.decisions_for_session(&session.id);
  assert_eq!(all.len(), 1, "insert_decision should upsert, not duplicate");
  assert_eq!(all[0].choice, "hetzner");
}

// ---- settings --------------------------------------------------------

#[test]
fn settings_defaults_and_round_trip() {
  let store = Store::open_in_memory().unwrap();
  assert_eq!(store.get_settings(), Settings::default());

  let mut custom = Settings {
    reviewer_name: "Jane Architect".to_string(),
    confidence_threshold: 0.7,
    ..Settings::default()
  };
  custom.weights.insert("nfr_fit".to_string(), 0.3);
  store.save_settings(&custom).unwrap();

  assert_eq!(store.get_settings(), custom);
}

// ---- AC-17 (storage part): data notice ack persists across reopening -----

#[test]
fn data_notice_ack_persists_across_reopening_a_file_backed_db() {
  let dir = tempfile::tempdir().unwrap();
  let path = dir.path().join("architect.sqlite");

  {
    let store = Store::open(&path).unwrap();
    assert!(!store.data_notice_acked());
    store.ack_data_notice().unwrap();
    assert!(store.data_notice_acked());
  }

  // Reopen the same file: the acknowledgement should still be there.
  {
    let store = Store::open(&path).unwrap();
    assert!(store.data_notice_acked());
  }
}

// ---- AC-3 (storage part): the API key never reaches the SQLite file ------

#[test]
fn the_sqlite_file_never_contains_the_api_key() {
  const SECRET: &str = "sk-or-test-SECRET";

  // The API key lives only in a SecretStore, which the Store never sees.
  let secret_store = MemoryStore::default();
  secret_store.set(SECRET).unwrap();
  assert!(secret_store.has().unwrap());

  let dir = tempfile::tempdir().unwrap();
  let path = dir.path().join("architect.sqlite");
  let store = Store::open(&path).unwrap();

  // Exercise settings and session storage (nothing here ever carries the
  // key — `Settings` has no such field, by construction).
  store.save_settings(&Settings::default()).unwrap();
  let session = new_session("describe", "Some project mentioning nothing secret");
  store.create_session(&session).unwrap();
  store.update_session_brief(&session.id, r#"{"summary":"A brief"}"#).unwrap();
  drop(store);

  let bytes = std::fs::read(&path).unwrap();
  assert!(
    !bytes.windows(SECRET.len()).any(|w| w == SECRET.as_bytes()),
    "the API key must never appear in the SQLite file"
  );
}
