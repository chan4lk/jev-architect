//! Serde DTOs for a decided outcome (spec FR-12, FR-14).
//!
//! Composition and routing (the arithmetic that fills these fields in) is
//! pure code living in `crate::scoring`; this module only carries the
//! shapes so `Store` and `render` can persist and render them.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Whether a decision is ready to ship as-is, or needs an architect's review
/// (spec FR-14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Route {
  Proposed,
  NeedsArchitect,
}

/// Why a decision was routed `NeedsArchitect` (spec FR-14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasonCode {
  LowConfidence,
  Disagreement,
  CloseMargin,
  HoldOption,
  PossibleInjection,
}

/// One option's per-criterion scores and the composite computed from them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OptionScore {
  pub option_id: String,
  pub criterion_scores: BTreeMap<String, f64>,
  pub composite: f64,
}

/// A decided outcome for one applicable decision type (spec FR-12, FR-14).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionResult {
  pub id: String,
  pub session_id: String,
  pub type_id: String,
  pub choice: String,
  pub confidence: f64,
  pub probabilities: BTreeMap<String, f64>,
  pub option_scores: Vec<OptionScore>,
  pub route: Route,
  pub reasons: Vec<ReasonCode>,
  pub model_snapshot: String,
  pub request_hash: String,
  pub cited_sections: Vec<String>,
}

/// A candidate decision type that scored below the applicability threshold
/// (spec FR-11): reported, but not decided.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NotApplicable {
  pub type_id: String,
  pub probability: f64,
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn route_and_reason_code_serialize_snake_case() {
    assert_eq!(serde_json::to_value(Route::NeedsArchitect).unwrap(), "needs_architect");
    assert_eq!(serde_json::to_value(Route::Proposed).unwrap(), "proposed");
    assert_eq!(serde_json::to_value(ReasonCode::LowConfidence).unwrap(), "low_confidence");
    assert_eq!(
      serde_json::to_value(ReasonCode::PossibleInjection).unwrap(),
      "possible_injection"
    );
  }

  #[test]
  fn decision_result_round_trips_through_json() {
    let d = DecisionResult {
      id: "d1".to_string(),
      session_id: "s1".to_string(),
      type_id: "cloud-platform".to_string(),
      choice: "azure".to_string(),
      confidence: 0.9,
      probabilities: BTreeMap::from([("azure".to_string(), 0.9), ("hetzner".to_string(), 0.1)]),
      option_scores: vec![OptionScore {
        option_id: "azure".to_string(),
        criterion_scores: BTreeMap::from([("nfr_fit".to_string(), 4.0)]),
        composite: 1.0,
      }],
      route: Route::Proposed,
      reasons: vec![],
      model_snapshot: "typesafe/jev-1.13-2026-01-01".to_string(),
      request_hash: "abc123".to_string(),
      cited_sections: vec!["S1".to_string()],
    };
    let json = serde_json::to_string(&d).unwrap();
    let back: DecisionResult = serde_json::from_str(&json).unwrap();
    assert_eq!(d, back);
  }
}
