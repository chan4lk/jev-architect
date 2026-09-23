//! Serde DTOs for the Bistec decision catalogue.
//!
//! These types mirror `catalog/decision-types.yaml`, `catalog/criteria.yaml`,
//! and `catalog/rules.yaml` verbatim. Loading, validation, and alias matching
//! live in `crate::catalog`; this module only carries the shapes.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Where an option sits on Bistec's adopt / trial / hold technology radar.
///
/// Ring mapping from the skill's matrix: Recommended -> adopt,
/// Alternative -> trial, Avoid -> hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Ring {
  Adopt,
  Trial,
  Hold,
}

/// One selectable technology for a decision type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptionDef {
  pub id: String,
  pub name: String,
  pub ring: Ring,
  pub description: String,
  #[serde(default)]
  pub aliases: Vec<String>,
}

/// A row of the skill's "Quick Reference: Technology Selection Matrix",
/// kept verbatim so `catalog-check` can diff it against the source skill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatrixRow {
  pub requirement: String,
  pub recommended: String,
  pub alternative: String,
  pub avoid: String,
}

/// One decision type: either a Technology Selection Matrix row, or one of
/// the two platform types (`cloud-platform`, `backend-platform`) decided
/// before the matrix rows (spec FR-9).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionType {
  pub id: String,
  pub name: String,
  #[serde(default)]
  pub group: Option<String>,
  #[serde(default)]
  pub source: Option<String>,
  #[serde(default)]
  pub matrix: Option<MatrixRow>,
  pub question: String,
  pub options: Vec<OptionDef>,
}

/// One scoring criterion: a 5-level Jev Score rubric (level 0 = worst, 4 = best).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Criterion {
  pub id: String,
  pub name: String,
  pub weight: f64,
  pub instructions: String,
  pub levels: Vec<String>,
}

/// One variant-selection rule group (spec FR-9). Rules only select or
/// exclude catalogue rows by `by`'s decided value; they never pick a
/// technology.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleGroup {
  pub group: String,
  pub by: String,
  pub select: BTreeMap<String, Vec<String>>,
}

/// All variant-selection rules plus the Auth Decision Tree text used as
/// Choice criteria for `auth_app_type`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rules {
  pub groups: Vec<RuleGroup>,
  pub auth_app_types: BTreeMap<String, String>,
}

/// The whole Bistec decision catalogue: decision types, scoring criteria,
/// variant-selection rules, and the ADR template.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Catalog {
  pub types: Vec<DecisionType>,
  pub criteria: Vec<Criterion>,
  pub rules: Rules,
  pub adr_template: String,
}
