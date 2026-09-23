//! SQLite persistence (spec FR-21, design's Data Model): sessions, stage
//! results (for resume + replay), decisions, append-only reviews, Jev call
//! usage, and settings. Migrations run via `rusqlite_migration`.
//!
//! `Store` wraps a single `rusqlite::Connection` behind a `Mutex` so it is
//! `Send + Sync` and can be shared as Tauri-managed state.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;

use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{params, Connection, OptionalExtension, Row};
use rusqlite_migration::{Migrations, M};
use serde_json::Value;
use thiserror::Error;

use crate::model::decision::{DecisionResult, OptionScore, ReasonCode, Route};
use crate::model::review::{validate_review, NewReview, Review, ReviewAction, ReviewError};
use crate::model::settings::Settings;

/// Errors from a `Store` operation.
#[derive(Debug, Error)]
pub enum StoreError {
  #[error("sqlite error: {0}")]
  Sqlite(#[from] rusqlite::Error),
  #[error("migration error: {0}")]
  Migration(#[from] rusqlite_migration::Error),
  #[error("json error: {0}")]
  Json(#[from] serde_json::Error),
  #[error("review validation failed: {0}")]
  Review(#[from] ReviewError),
}

/// A stored session: the input, its brief, and its pipeline progress
/// (design's `sessions` table).
#[derive(Debug, Clone, PartialEq)]
pub struct SessionRow {
  pub id: String,
  pub created_at: DateTime<Utc>,
  pub title: String,
  /// `"describe"` (Mode A) or `"upload"` (Mode B).
  pub mode: String,
  pub input_text: Option<String>,
  pub doc_name: Option<String>,
  pub sections_json: Option<String>,
  pub brief_json: Option<String>,
  pub brief_confirmed_at: Option<DateTime<Utc>>,
  /// `"brief"` | `"gates"` | … | `"done"` | `"failed:<stage>"`.
  pub stage: String,
  pub error: Option<String>,
}

/// One accepted or overridden past decision of the same type, offered to
/// Jev as `bistec_precedent` (spec FR-13).
#[derive(Debug, Clone, PartialEq)]
pub struct Precedent {
  pub type_id: String,
  pub option_id: String,
  pub summary: String,
  pub at_utc: DateTime<Utc>,
}

/// The store's schema, as a single migration (see the design's Data Model
/// table). Multiple statements in one `M::up` run together as a batch.
fn migrations() -> Migrations<'static> {
  Migrations::new(vec![M::up(
    r#"
    CREATE TABLE settings (
      key   TEXT PRIMARY KEY,
      value TEXT NOT NULL
    );

    CREATE TABLE sessions (
      id                  TEXT PRIMARY KEY,
      created_at          TEXT NOT NULL,
      title               TEXT NOT NULL,
      mode                TEXT NOT NULL,
      input_text          TEXT,
      doc_name            TEXT,
      sections_json       TEXT,
      brief_json          TEXT,
      brief_confirmed_at  TEXT,
      stage               TEXT NOT NULL,
      error               TEXT
    );

    CREATE TABLE stage_results (
      session_id  TEXT NOT NULL,
      stage       TEXT NOT NULL,
      result_json TEXT NOT NULL,
      created_at  TEXT NOT NULL,
      PRIMARY KEY (session_id, stage)
    );

    CREATE TABLE decisions (
      id                   TEXT PRIMARY KEY,
      session_id           TEXT NOT NULL,
      type_id              TEXT NOT NULL,
      choice_json          TEXT NOT NULL,
      scores_json          TEXT NOT NULL,
      composite_json       TEXT NOT NULL,
      route                TEXT NOT NULL,
      reasons_json         TEXT NOT NULL,
      model_snapshot       TEXT NOT NULL,
      request_hash         TEXT NOT NULL,
      cited_sections_json  TEXT NOT NULL,
      adr_number           INTEGER
    );

    CREATE TABLE reviews (
      id           INTEGER PRIMARY KEY AUTOINCREMENT,
      decision_id  TEXT NOT NULL,
      action       TEXT NOT NULL,
      option_id    TEXT,
      reviewer     TEXT NOT NULL,
      reason       TEXT,
      at_utc       TEXT NOT NULL
    );

    -- Append-only (spec AC-13): a later review supersedes an earlier one,
    -- it never replaces or removes it.
    CREATE TRIGGER reviews_no_update
      BEFORE UPDATE ON reviews
      BEGIN
        SELECT RAISE(ABORT, 'reviews is append-only: UPDATE is not allowed');
      END;

    CREATE TRIGGER reviews_no_delete
      BEFORE DELETE ON reviews
      BEGIN
        SELECT RAISE(ABORT, 'reviews is append-only: DELETE is not allowed');
      END;

    CREATE TABLE jev_calls (
      id             INTEGER PRIMARY KEY AUTOINCREMENT,
      session_id     TEXT NOT NULL,
      stage          TEXT NOT NULL,
      input_tokens   INTEGER NOT NULL,
      output_tokens  INTEGER NOT NULL,
      cost_usd       REAL,
      model_snapshot TEXT NOT NULL,
      at_utc         TEXT NOT NULL
    );
    "#,
  )])
}

/// RFC 3339 with a fixed nanosecond width, so lexicographic order on the
/// stored text matches chronological order.
fn dt_to_text(dt: DateTime<Utc>) -> String {
  dt.to_rfc3339_opts(SecondsFormat::Nanos, true)
}

fn text_to_dt(s: &str) -> DateTime<Utc> {
  DateTime::parse_from_rfc3339(s)
    .unwrap_or_else(|e| panic!("stored timestamp {s:?} should be valid RFC3339: {e}"))
    .with_timezone(&Utc)
}

fn action_to_text(action: ReviewAction) -> String {
  serde_json::to_value(action)
    .expect("ReviewAction always serializes")
    .as_str()
    .expect("ReviewAction serializes to a string")
    .to_string()
}

fn action_from_text(s: &str) -> ReviewAction {
  serde_json::from_value(Value::String(s.to_string()))
    .unwrap_or_else(|e| panic!("stored review action {s:?} should be valid: {e}"))
}

fn route_to_text(route: Route) -> String {
  serde_json::to_value(route)
    .expect("Route always serializes")
    .as_str()
    .expect("Route serializes to a string")
    .to_string()
}

fn route_from_text(s: &str) -> Route {
  serde_json::from_value(Value::String(s.to_string())).unwrap_or_else(|e| panic!("stored route {s:?} should be valid: {e}"))
}

fn row_to_session(row: &Row) -> rusqlite::Result<SessionRow> {
  let created_at: String = row.get(1)?;
  let brief_confirmed_at: Option<String> = row.get(8)?;
  Ok(SessionRow {
    id: row.get(0)?,
    created_at: text_to_dt(&created_at),
    title: row.get(2)?,
    mode: row.get(3)?,
    input_text: row.get(4)?,
    doc_name: row.get(5)?,
    sections_json: row.get(6)?,
    brief_json: row.get(7)?,
    brief_confirmed_at: brief_confirmed_at.as_deref().map(text_to_dt),
    stage: row.get(9)?,
    error: row.get(10)?,
  })
}

const SESSION_COLUMNS: &str =
  "id, created_at, title, mode, input_text, doc_name, sections_json, brief_json, brief_confirmed_at, stage, error";

fn row_to_decision(row: &Row) -> rusqlite::Result<DecisionResult> {
  let choice_json: String = row.get(3)?;
  let scores_json: String = row.get(4)?;
  let route_text: String = row.get(6)?;
  let reasons_json: String = row.get(7)?;
  let cited_sections_json: String = row.get(10)?;

  let choice_val: Value =
    serde_json::from_str(&choice_json).unwrap_or_else(|e| panic!("stored choice_json should be valid JSON: {e}"));
  let option_scores: Vec<OptionScore> =
    serde_json::from_str(&scores_json).unwrap_or_else(|e| panic!("stored scores_json should be valid JSON: {e}"));
  let reasons: Vec<ReasonCode> =
    serde_json::from_str(&reasons_json).unwrap_or_else(|e| panic!("stored reasons_json should be valid JSON: {e}"));
  let cited_sections: Vec<String> = serde_json::from_str(&cited_sections_json)
    .unwrap_or_else(|e| panic!("stored cited_sections_json should be valid JSON: {e}"));

  Ok(DecisionResult {
    id: row.get(0)?,
    session_id: row.get(1)?,
    type_id: row.get(2)?,
    choice: choice_val["choice"]
      .as_str()
      .unwrap_or_else(|| panic!("choice_json.choice should be a string, got {choice_val:?}"))
      .to_string(),
    confidence: choice_val["confidence"]
      .as_f64()
      .unwrap_or_else(|| panic!("choice_json.confidence should be a number, got {choice_val:?}")),
    probabilities: serde_json::from_value(choice_val["probabilities"].clone())
      .unwrap_or_else(|e| panic!("choice_json.probabilities should be an object: {e}")),
    option_scores,
    route: route_from_text(&route_text),
    reasons,
    model_snapshot: row.get(8)?,
    request_hash: row.get(9)?,
    cited_sections,
  })
}

const DECISION_COLUMNS: &str = "id, session_id, type_id, choice_json, scores_json, composite_json, route, reasons_json, \
model_snapshot, request_hash, cited_sections_json";

fn row_to_review(row: &Row) -> rusqlite::Result<Review> {
  let action_text: String = row.get(2)?;
  let at_utc: String = row.get(6)?;
  Ok(Review {
    id: row.get(0)?,
    decision_id: row.get(1)?,
    action: action_from_text(&action_text),
    option_id: row.get(3)?,
    reviewer: row.get(4)?,
    reason: row.get(5)?,
    at_utc: text_to_dt(&at_utc),
  })
}

/// The latest review by `(at_utc, id)`, as used by both `adr_status` and
/// `precedent` (spec FR-13, FR-18).
fn latest_review(reviews: &[Review]) -> Option<&Review> {
  reviews.iter().max_by_key(|r| (r.at_utc, r.id))
}

fn brief_summary_or_title(session: &SessionRow) -> String {
  if let Some(brief_json) = &session.brief_json {
    if let Ok(value) = serde_json::from_str::<Value>(brief_json) {
      if let Some(summary) = value.get("summary").and_then(Value::as_str) {
        return summary.to_string();
      }
    }
  }
  session.title.clone()
}

pub struct Store {
  conn: Mutex<Connection>,
}

impl Store {
  /// Opens (creating if needed) a file-backed store at `path` and applies
  /// migrations up to the latest version.
  pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
    let mut conn = Connection::open(path)?;
    migrations().to_latest(&mut conn)?;
    Ok(Store { conn: Mutex::new(conn) })
  }

  /// Opens an in-memory store (tests) and applies migrations.
  pub fn open_in_memory() -> Result<Self, StoreError> {
    let mut conn = Connection::open_in_memory()?;
    migrations().to_latest(&mut conn)?;
    Ok(Store { conn: Mutex::new(conn) })
  }

  fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
    self.conn.lock().expect("store mutex poisoned")
  }

  // ---- sessions ---------------------------------------------------------

  pub fn create_session(&self, row: &SessionRow) -> Result<(), StoreError> {
    self.conn().execute(
      &format!("INSERT INTO sessions ({SESSION_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"),
      params![
        row.id,
        dt_to_text(row.created_at),
        row.title,
        row.mode,
        row.input_text,
        row.doc_name,
        row.sections_json,
        row.brief_json,
        row.brief_confirmed_at.map(dt_to_text),
        row.stage,
        row.error,
      ],
    )?;
    Ok(())
  }

  pub fn get_session(&self, id: &str) -> Option<SessionRow> {
    self
      .conn()
      .query_row(
        &format!("SELECT {SESSION_COLUMNS} FROM sessions WHERE id = ?1"),
        params![id],
        row_to_session,
      )
      .optional()
      .expect("get_session query failed")
  }

  /// Every session, newest first.
  pub fn list_sessions(&self) -> Vec<SessionRow> {
    let conn = self.conn();
    let mut stmt = conn
      .prepare(&format!("SELECT {SESSION_COLUMNS} FROM sessions ORDER BY created_at DESC, id DESC"))
      .expect("prepare list_sessions failed");
    let rows = stmt.query_map([], row_to_session).expect("query list_sessions failed");
    rows.map(|r| r.expect("session row decode failed")).collect()
  }

  pub fn update_session_brief(&self, id: &str, brief_json: &str) -> Result<(), StoreError> {
    self
      .conn()
      .execute("UPDATE sessions SET brief_json = ?1 WHERE id = ?2", params![brief_json, id])?;
    Ok(())
  }

  /// Sets `brief_confirmed_at = now()` (spec FR-8): nothing reaches the
  /// decision pass without this.
  pub fn confirm_brief(&self, id: &str) -> Result<(), StoreError> {
    self.conn().execute(
      "UPDATE sessions SET brief_confirmed_at = ?1 WHERE id = ?2",
      params![dt_to_text(Utc::now()), id],
    )?;
    Ok(())
  }

  pub fn set_session_stage(&self, id: &str, stage: &str, error: Option<&str>) -> Result<(), StoreError> {
    self
      .conn()
      .execute("UPDATE sessions SET stage = ?1, error = ?2 WHERE id = ?3", params![stage, error, id])?;
    Ok(())
  }

  pub fn set_sections(&self, id: &str, sections_json: &str) -> Result<(), StoreError> {
    self
      .conn()
      .execute("UPDATE sessions SET sections_json = ?1 WHERE id = ?2", params![sections_json, id])?;
    Ok(())
  }

  // ---- stage results ------------------------------------------------------

  /// Upserts a stage's persisted result, keyed by `(session_id, stage)`
  /// (drives Retry-from-stage and History replay; spec AC-16).
  pub fn put_stage_result(&self, session_id: &str, stage: &str, result: &Value) -> Result<(), StoreError> {
    let json = serde_json::to_string(result)?;
    self.conn().execute(
      "INSERT INTO stage_results (session_id, stage, result_json, created_at) VALUES (?1, ?2, ?3, ?4)
       ON CONFLICT(session_id, stage) DO UPDATE SET result_json = excluded.result_json, created_at = excluded.created_at",
      params![session_id, stage, json, dt_to_text(Utc::now())],
    )?;
    Ok(())
  }

  pub fn get_stage_result(&self, session_id: &str, stage: &str) -> Option<Value> {
    let json: Option<String> = self
      .conn()
      .query_row(
        "SELECT result_json FROM stage_results WHERE session_id = ?1 AND stage = ?2",
        params![session_id, stage],
        |row| row.get(0),
      )
      .optional()
      .expect("get_stage_result query failed");
    json.map(|j| serde_json::from_str(&j).unwrap_or_else(|e| panic!("stored stage result should be valid JSON: {e}")))
  }

  /// Deletes any persisted result for `stages` of this session (used before
  /// re-running from an earlier stage).
  pub fn clear_stage_results_from(&self, session_id: &str, stages: &[&str]) -> Result<(), StoreError> {
    let conn = self.conn();
    for stage in stages {
      conn.execute("DELETE FROM stage_results WHERE session_id = ?1 AND stage = ?2", params![session_id, stage])?;
    }
    Ok(())
  }

  // ---- decisions ------------------------------------------------------

  /// Upserts a decision by id. Deliberately does not touch `adr_number`:
  /// re-inserting a decision (e.g. a retried stage) keeps whatever ADR
  /// number a previous export already assigned it.
  pub fn insert_decision(&self, d: &DecisionResult) -> Result<(), StoreError> {
    let choice_json = serde_json::to_string(&serde_json::json!({
      "choice": d.choice,
      "confidence": d.confidence,
      "probabilities": d.probabilities,
    }))?;
    let scores_json = serde_json::to_string(&d.option_scores)?;
    let composite_json = serde_json::to_string(
      &d.option_scores
        .iter()
        .map(|s| (s.option_id.clone(), s.composite))
        .collect::<BTreeMap<_, _>>(),
    )?;
    let reasons_json = serde_json::to_string(&d.reasons)?;
    let cited_sections_json = serde_json::to_string(&d.cited_sections)?;

    self.conn().execute(
      &format!(
        "INSERT INTO decisions ({DECISION_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
         ON CONFLICT(id) DO UPDATE SET
           session_id = excluded.session_id,
           type_id = excluded.type_id,
           choice_json = excluded.choice_json,
           scores_json = excluded.scores_json,
           composite_json = excluded.composite_json,
           route = excluded.route,
           reasons_json = excluded.reasons_json,
           model_snapshot = excluded.model_snapshot,
           request_hash = excluded.request_hash,
           cited_sections_json = excluded.cited_sections_json"
      ),
      params![
        d.id,
        d.session_id,
        d.type_id,
        choice_json,
        scores_json,
        composite_json,
        route_to_text(d.route),
        reasons_json,
        d.model_snapshot,
        d.request_hash,
        cited_sections_json,
      ],
    )?;
    Ok(())
  }

  pub fn decisions_for_session(&self, session_id: &str) -> Vec<DecisionResult> {
    let conn = self.conn();
    let mut stmt = conn
      .prepare(&format!("SELECT {DECISION_COLUMNS} FROM decisions WHERE session_id = ?1 ORDER BY id"))
      .expect("prepare decisions_for_session failed");
    let rows = stmt
      .query_map(params![session_id], row_to_decision)
      .expect("query decisions_for_session failed");
    rows.map(|r| r.expect("decision row decode failed")).collect()
  }

  pub fn set_adr_number(&self, decision_id: &str, n: u32) -> Result<(), StoreError> {
    self
      .conn()
      .execute("UPDATE decisions SET adr_number = ?1 WHERE id = ?2", params![n, decision_id])?;
    Ok(())
  }

  // ---- reviews (append-only) ------------------------------------------

  /// Validates and records a new review (spec FR-18, AC-13). Never updates
  /// or deletes an existing row — the `reviews` table's triggers also
  /// enforce this at the DB level.
  pub fn append_review(&self, r: &NewReview) -> Result<Review, StoreError> {
    validate_review(r)?;
    let at_utc = Utc::now();
    let conn = self.conn();
    conn.execute(
      "INSERT INTO reviews (decision_id, action, option_id, reviewer, reason, at_utc) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
      params![r.decision_id, action_to_text(r.action), r.option_id, r.reviewer, r.reason, dt_to_text(at_utc)],
    )?;
    let id = conn.last_insert_rowid();
    Ok(Review {
      id,
      decision_id: r.decision_id.clone(),
      action: r.action,
      option_id: r.option_id.clone(),
      reviewer: r.reviewer.clone(),
      reason: r.reason.clone(),
      at_utc,
    })
  }

  /// Every review of a decision, oldest first.
  pub fn reviews_for_decision(&self, decision_id: &str) -> Vec<Review> {
    let conn = self.conn();
    let mut stmt = conn
      .prepare("SELECT id, decision_id, action, option_id, reviewer, reason, at_utc FROM reviews WHERE decision_id = ?1 ORDER BY at_utc ASC, id ASC")
      .expect("prepare reviews_for_decision failed");
    let rows = stmt
      .query_map(params![decision_id], row_to_review)
      .expect("query reviews_for_decision failed");
    rows.map(|r| r.expect("review row decode failed")).collect()
  }

  // ---- Jev call usage ---------------------------------------------------

  pub fn log_jev_call(
    &self,
    session_id: &str,
    stage: &str,
    input_tokens: u64,
    output_tokens: u64,
    cost_usd: Option<f64>,
    model_snapshot: &str,
  ) -> Result<(), StoreError> {
    self.conn().execute(
      "INSERT INTO jev_calls (session_id, stage, input_tokens, output_tokens, cost_usd, model_snapshot, at_utc)
       VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
      params![
        session_id,
        stage,
        input_tokens as i64,
        output_tokens as i64,
        cost_usd,
        model_snapshot,
        dt_to_text(Utc::now()),
      ],
    )?;
    Ok(())
  }

  /// Total input tokens, and the sum of every call's cost — or `None` if
  /// no call in this session reported a cost (spec FR-16, edge case "cost
  /// missing").
  pub fn session_usage(&self, session_id: &str) -> (u64, Option<f64>) {
    let (input_tokens, cost_sum, cost_count): (Option<i64>, Option<f64>, i64) = self
      .conn()
      .query_row(
        "SELECT SUM(input_tokens), SUM(cost_usd), COUNT(cost_usd) FROM jev_calls WHERE session_id = ?1",
        params![session_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
      )
      .expect("session_usage query failed");

    let total_input = input_tokens.unwrap_or(0).max(0) as u64;
    let cost = if cost_count > 0 { cost_sum } else { None };
    (total_input, cost)
  }

  // ---- precedent ---------------------------------------------------------

  /// Up to `limit` of the most recent Accepted/Overridden decisions of
  /// `type_id` (spec FR-13, AC-12), newest review first. A decision whose
  /// latest review is Reject (or that has no review at all) is excluded.
  pub fn precedent(&self, type_id: &str, limit: usize) -> Vec<Precedent> {
    let decisions: Vec<(String, String, String)> = {
      let conn = self.conn();
      let mut stmt = conn
        .prepare("SELECT id, session_id, choice_json FROM decisions WHERE type_id = ?1")
        .expect("prepare precedent decisions query failed");
      let rows = stmt
        .query_map(params![type_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .expect("query precedent decisions failed");
      rows.map(|r| r.expect("decision row decode failed")).collect()
    };

    let mut candidates: Vec<Precedent> = Vec::new();
    for (decision_id, session_id, choice_json) in decisions {
      let reviews = self.reviews_for_decision(&decision_id);
      let Some(latest) = latest_review(&reviews) else {
        continue;
      };

      let option_id = match latest.action {
        ReviewAction::Accept => {
          let choice_val: Value =
            serde_json::from_str(&choice_json).unwrap_or_else(|e| panic!("stored choice_json should be valid JSON: {e}"));
          choice_val["choice"]
            .as_str()
            .unwrap_or_else(|| panic!("choice_json.choice should be a string, got {choice_val:?}"))
            .to_string()
        }
        ReviewAction::Override => latest
          .option_id
          .clone()
          .expect("an Override review always has an option_id (validate_review enforces this)"),
        ReviewAction::Reject => continue,
      };

      let summary = self.get_session(&session_id).map(|s| brief_summary_or_title(&s)).unwrap_or_default();

      candidates.push(Precedent {
        type_id: type_id.to_string(),
        option_id,
        summary,
        at_utc: latest.at_utc,
      });
    }

    candidates.sort_by(|a, b| b.at_utc.cmp(&a.at_utc).then_with(|| b.type_id.cmp(&a.type_id)));
    candidates.truncate(limit);
    candidates
  }

  // ---- settings + data notice ------------------------------------------

  pub fn get_settings(&self) -> Settings {
    let json: Option<String> = self
      .conn()
      .query_row("SELECT value FROM settings WHERE key = 'settings'", [], |row| row.get(0))
      .optional()
      .expect("get_settings query failed");
    match json {
      Some(j) => serde_json::from_str(&j).unwrap_or_else(|e| panic!("stored settings should be valid JSON: {e}")),
      None => Settings::default(),
    }
  }

  pub fn save_settings(&self, settings: &Settings) -> Result<(), StoreError> {
    let json = serde_json::to_string(settings)?;
    self.conn().execute(
      "INSERT INTO settings (key, value) VALUES ('settings', ?1)
       ON CONFLICT(key) DO UPDATE SET value = excluded.value",
      params![json],
    )?;
    Ok(())
  }

  /// Whether the first-run data notice (spec FR-3) has been acknowledged.
  pub fn data_notice_acked(&self) -> bool {
    let value: Option<String> = self
      .conn()
      .query_row("SELECT value FROM settings WHERE key = 'data_notice_acked'", [], |row| row.get(0))
      .optional()
      .expect("data_notice_acked query failed");
    value.as_deref() == Some("true")
  }

  pub fn ack_data_notice(&self) -> Result<(), StoreError> {
    self.conn().execute(
      "INSERT INTO settings (key, value) VALUES ('data_notice_acked', 'true')
       ON CONFLICT(key) DO UPDATE SET value = excluded.value",
      [],
    )?;
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use chrono::TimeZone;

  #[test]
  fn fresh_in_memory_store_opens_and_migrates() {
    let store = Store::open_in_memory().expect("fresh store should migrate cleanly");
    // A trivial round trip proves the schema is actually usable.
    assert_eq!(store.get_settings(), Settings::default());
    assert!(!store.data_notice_acked());
  }

  #[test]
  fn dt_text_round_trip_preserves_the_instant() {
    let now = Utc::now();
    assert_eq!(text_to_dt(&dt_to_text(now)), now);
  }

  #[test]
  fn dt_text_sorts_lexicographically_in_chronological_order() {
    let earlier = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let later = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 1).unwrap();
    assert!(dt_to_text(earlier) < dt_to_text(later));
  }

  /// A trigger enforces append-only at the DB level (spec AC-13, key
  /// decision 6), independent of code convention. This needs the private
  /// `Connection` to issue raw SQL directly, so it lives here rather than
  /// in `tests/store.rs`.
  #[test]
  fn reviews_table_rejects_raw_update_and_delete() {
    let store = Store::open_in_memory().unwrap();
    store
      .create_session(&SessionRow {
        id: "s1".to_string(),
        created_at: Utc::now(),
        title: "Proj".to_string(),
        mode: "describe".to_string(),
        input_text: None,
        doc_name: None,
        sections_json: None,
        brief_json: None,
        brief_confirmed_at: None,
        stage: "brief".to_string(),
        error: None,
      })
      .unwrap();
    store
      .insert_decision(&DecisionResult {
        id: "d1".to_string(),
        session_id: "s1".to_string(),
        type_id: "cloud-platform".to_string(),
        choice: "azure".to_string(),
        confidence: 0.8,
        probabilities: BTreeMap::from([("azure".to_string(), 0.8)]),
        option_scores: vec![],
        route: Route::Proposed,
        reasons: vec![],
        model_snapshot: "typesafe/jev-1.13-2026-01-01".to_string(),
        request_hash: "hash".to_string(),
        cited_sections: vec![],
      })
      .unwrap();
    store
      .append_review(&NewReview {
        decision_id: "d1".to_string(),
        action: ReviewAction::Accept,
        option_id: None,
        reviewer: "Jane".to_string(),
        reason: None,
      })
      .unwrap();

    let conn = store.conn();
    let update_result = conn.execute("UPDATE reviews SET reviewer = 'Someone Else'", ());
    assert!(update_result.is_err(), "raw UPDATE on reviews should be rejected by the trigger");

    let delete_result = conn.execute("DELETE FROM reviews", ());
    assert!(delete_result.is_err(), "raw DELETE on reviews should be rejected by the trigger");
  }
}
