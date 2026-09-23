//! Report view DTOs — the `Report` / `DecisionView` shapes of the IPC contract
//! (.specclaw/changes/001-bistec-architect-agent/ipc-contract.md). Assembled by
//! `pipeline::build_report`, rendered by `render`, sent to the UI by `commands`.

use serde::{Deserialize, Serialize};

use crate::model::catalog::Ring;
use crate::model::decision::DecisionResult;
use crate::model::review::{AdrStatus, Review};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct OptionView {
  pub id: String,
  pub name: String,
  pub ring: Ring,
  pub description: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DecisionView {
  pub decision: DecisionResult,
  pub type_name: String,
  pub options: Vec<OptionView>,
  pub reviews: Vec<Review>,
  pub status: AdrStatus,
  pub status_label: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct NotApplicableView {
  pub type_id: String,
  pub type_name: String,
  pub probability: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Gates {
  pub is_technical_request: f64,
  pub has_enough_context: f64,
  pub injection: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct CriterionView {
  pub id: String,
  pub name: String,
  pub weight: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Report {
  pub gates: Gates,
  pub decisions: Vec<DecisionView>,
  pub not_applicable: Vec<NotApplicableView>,
  pub input_tokens: u64,
  pub cost_usd: Option<f64>,
  pub truncated: bool,
  pub criteria: Vec<CriterionView>,
}
