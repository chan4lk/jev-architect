//! Loads, validates, and queries the Bistec decision catalogue.
//!
//! The catalogue is bundled into the binary at compile time via
//! `include_str!` (spec NFR-8 keeps it offline-capable and dependency-free at
//! runtime). `catalog-check` (FR-22) reuses [`diff_against_skill`] to compare
//! the bundled catalogue against a live `SKILL.md`.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt;

use serde::Deserialize;
use thiserror::Error;

use crate::model::catalog::{Criterion, DecisionType, MatrixRow, OptionDef, Ring, Rules};

pub use crate::model::catalog::Catalog;

const TYPES_YAML: &str = include_str!("../../catalog/decision-types.yaml");
const CRITERIA_YAML: &str = include_str!("../../catalog/criteria.yaml");
const RULES_YAML: &str = include_str!("../../catalog/rules.yaml");
const ADR_TEMPLATE: &str = include_str!("../../catalog/adr-template.md");

const SKILL_MATRIX_HEADING: &str = "## Quick Reference: Technology Selection Matrix";

/// Errors from loading, validating, or diffing the catalogue.
#[derive(Debug, Error)]
pub enum CatalogError {
  #[error("failed to parse {file}: {source}")]
  Yaml {
    file: &'static str,
    #[source]
    source: serde_yaml::Error,
  },
  #[error("duplicate decision type id: {0}")]
  DuplicateTypeId(String),
  #[error("decision type {type_id}: duplicate option id: {option_id}")]
  DuplicateOptionId { type_id: String, option_id: String },
  #[error("decision type {type_id}: must have 1..=255 options, has {count}")]
  OptionCountOutOfRange { type_id: String, count: usize },
  #[error("criterion {id}: must have exactly 5 levels, has {count}")]
  CriterionLevelCount { id: String, count: usize },
  #[error("criterion {id}: weight must be > 0, was {weight}")]
  CriterionWeightNotPositive { id: String, weight: f64 },
  #[error("rules.yaml group '{group}' references unknown decision type: {type_id}")]
  UnknownRuleType { group: String, type_id: String },
  #[error(
    "decision type {type_id}: matrix '{column}' cell implies option name(s) {expected:?} with ring {ring:?}, but the type's {ring:?} option names are {actual:?}"
  )]
  MatrixMismatch {
    type_id: String,
    column: &'static str,
    ring: Ring,
    expected: Vec<String>,
    actual: Vec<String>,
  },
  #[error("SKILL.md has no '{heading}' heading", heading = SKILL_MATRIX_HEADING)]
  SkillHeadingNotFound,
  #[error("no markdown table found under '{heading}'", heading = SKILL_MATRIX_HEADING)]
  SkillTableNotFound,
  #[error("SKILL.md matrix table row has {count} cell(s), expected 4: {line}")]
  SkillTableRowShape { count: usize, line: String },
}

/// Deserialization shape of `catalog/decision-types.yaml` (the `version`
/// field is intentionally not modeled; serde ignores it).
#[derive(Deserialize)]
struct TypesFile {
  types: Vec<DecisionType>,
}

/// Deserialization shape of `catalog/criteria.yaml`.
#[derive(Deserialize)]
struct CriteriaFile {
  criteria: Vec<Criterion>,
}

impl Catalog {
  /// Loads and validates the catalogue bundled into this binary.
  pub fn bundled() -> Result<Catalog, CatalogError> {
    Catalog::from_strs(TYPES_YAML, CRITERIA_YAML, RULES_YAML, ADR_TEMPLATE)
  }

  /// Loads and validates a catalogue from raw YAML/Markdown strings.
  pub fn from_strs(
    types_yaml: &str,
    criteria_yaml: &str,
    rules_yaml: &str,
    adr_template: &str,
  ) -> Result<Catalog, CatalogError> {
    let types_file: TypesFile = serde_yaml::from_str(types_yaml).map_err(|source| CatalogError::Yaml {
      file: "decision-types.yaml",
      source,
    })?;
    let criteria_file: CriteriaFile = serde_yaml::from_str(criteria_yaml).map_err(|source| CatalogError::Yaml {
      file: "criteria.yaml",
      source,
    })?;
    let rules: Rules = serde_yaml::from_str(rules_yaml).map_err(|source| CatalogError::Yaml {
      file: "rules.yaml",
      source,
    })?;

    let catalog = Catalog {
      types: types_file.types,
      criteria: criteria_file.criteria,
      rules,
      adr_template: adr_template.to_string(),
    };
    catalog.validate()?;
    Ok(catalog)
  }

  /// Looks up a decision type by id.
  pub fn type_by_id(&self, id: &str) -> Option<&DecisionType> {
    self.types.iter().find(|t| t.id == id)
  }

  /// Returns each criterion's weight, keyed by criterion id.
  pub fn criterion_weights(&self) -> BTreeMap<String, f64> {
    self.criteria.iter().map(|c| (c.id.clone(), c.weight)).collect()
  }

  /// Validates internal consistency of the catalogue (AC-2, AC-11):
  /// unique type ids; unique option ids within a type; 1..=255 options per
  /// type; every criterion has exactly 5 levels and a positive weight; every
  /// type id referenced by `rules.yaml` exists; and every matrix type's
  /// options cover the split cells of its own matrix row.
  fn validate(&self) -> Result<(), CatalogError> {
    let mut seen_type_ids = BTreeSet::new();
    for t in &self.types {
      if !seen_type_ids.insert(t.id.clone()) {
        return Err(CatalogError::DuplicateTypeId(t.id.clone()));
      }

      let mut seen_option_ids = BTreeSet::new();
      for o in &t.options {
        if !seen_option_ids.insert(o.id.clone()) {
          return Err(CatalogError::DuplicateOptionId {
            type_id: t.id.clone(),
            option_id: o.id.clone(),
          });
        }
      }

      if t.options.is_empty() || t.options.len() > 255 {
        return Err(CatalogError::OptionCountOutOfRange {
          type_id: t.id.clone(),
          count: t.options.len(),
        });
      }

      if let Some(matrix) = &t.matrix {
        validate_matrix_coverage(t, matrix)?;
      }
    }

    for c in &self.criteria {
      if c.levels.len() != 5 {
        return Err(CatalogError::CriterionLevelCount {
          id: c.id.clone(),
          count: c.levels.len(),
        });
      }
      // Written as a negated `>` (not `<=`) so a NaN weight is also rejected.
      #[allow(clippy::neg_cmp_op_on_partial_ord)]
      if !(c.weight > 0.0) {
        return Err(CatalogError::CriterionWeightNotPositive {
          id: c.id.clone(),
          weight: c.weight,
        });
      }
    }

    for group in &self.rules.groups {
      for ids in group.select.values() {
        for id in ids {
          if self.type_by_id(id).is_none() {
            return Err(CatalogError::UnknownRuleType {
              group: group.group.clone(),
              type_id: id.clone(),
            });
          }
        }
      }
    }

    Ok(())
  }
}

/// Checks that a matrix type's adopt/trial/hold option names are exactly the
/// texts obtained by splitting the matrix row's recommended/alternative/avoid
/// cells (spec A2): "A / B" and "A, B" become two names; "—" contributes none.
fn validate_matrix_coverage(t: &DecisionType, matrix: &MatrixRow) -> Result<(), CatalogError> {
  check_ring_coverage(t, matrix, "Recommended", &matrix.recommended, Ring::Adopt)?;
  check_ring_coverage(t, matrix, "Alternative", &matrix.alternative, Ring::Trial)?;
  check_ring_coverage(t, matrix, "Avoid", &matrix.avoid, Ring::Hold)?;
  Ok(())
}

fn check_ring_coverage(
  t: &DecisionType,
  _matrix: &MatrixRow,
  column: &'static str,
  cell: &str,
  ring: Ring,
) -> Result<(), CatalogError> {
  let expected: BTreeSet<String> = split_cell(cell).into_iter().collect();
  let actual: BTreeSet<String> = t
    .options
    .iter()
    .filter(|o| o.ring == ring)
    .map(|o| o.name.clone())
    .collect();

  if expected != actual {
    return Err(CatalogError::MatrixMismatch {
      type_id: t.id.clone(),
      column,
      ring,
      expected: expected.into_iter().collect(),
      actual: actual.into_iter().collect(),
    });
  }
  Ok(())
}

/// Splits a matrix cell into the option-name texts it lists. Cells separate
/// several technologies with " / " or ", "; a "—" (or empty) cell lists none.
fn split_cell(cell: &str) -> Vec<String> {
  let trimmed = cell.trim();
  if trimmed.is_empty() || trimmed == "—" {
    return Vec::new();
  }

  let mut parts = Vec::new();
  for chunk in trimmed.split(" / ") {
    for sub in chunk.split(", ") {
      let sub = sub.trim();
      if !sub.is_empty() {
        parts.push(sub.to_string());
      }
    }
  }
  parts
}

/// True if `c` is a "word" character for the purposes of alias matching:
/// alphanumeric or underscore. A `.` (as in the alias `.net`) is therefore
/// *not* a word character, so it behaves as its own delimiter.
fn is_word_char(c: char) -> bool {
  c.is_alphanumeric() || c == '_'
}

/// True if `needle` occurs in `haystack` at a position where the character
/// immediately before and after the match (if any) is not a word character.
///
/// This is deliberately not a plain substring search: "node" must not match
/// inside "nodejs" (no boundary after "node"), which is the rule this
/// module documents and tests. It also is not a naive `\b`-anchored regex,
/// because that would fail on aliases that start or end with a non-word
/// character such as ".net" (a preceding space and a leading "." are both
/// non-word, so a `\b` immediately before "." never matches there). Instead,
/// a match is accepted whenever the characters flanking it are absent or are
/// themselves non-word — regardless of what the needle's own edge
/// characters are.
fn contains_whole_word(haystack: &str, needle: &str) -> bool {
  if needle.is_empty() {
    return false;
  }
  let hay: Vec<char> = haystack.chars().collect();
  let ndl: Vec<char> = needle.chars().collect();
  if ndl.len() > hay.len() {
    return false;
  }

  for start in 0..=(hay.len() - ndl.len()) {
    if hay[start..start + ndl.len()] == ndl[..] {
      let before_ok = start == 0 || !is_word_char(hay[start - 1]);
      let end = start + ndl.len();
      let after_ok = end == hay.len() || !is_word_char(hay[end]);
      if before_ok && after_ok {
        return true;
      }
    }
  }
  false
}

/// True if `opt` is mentioned in `mentions`: any of its aliases, or its own
/// name, appears case-insensitively as a whole word/phrase in any mention
/// (spec FR-12's hold-option eligibility check).
pub fn option_mentioned(opt: &OptionDef, mentions: &[String]) -> bool {
  let mut needles: Vec<String> = opt.aliases.clone();
  needles.push(opt.name.clone());

  for mention in mentions {
    let hay = mention.to_lowercase();
    for needle in &needles {
      let needle = needle.to_lowercase();
      if !needle.is_empty() && contains_whole_word(&hay, &needle) {
        return true;
      }
    }
  }
  false
}

/// One difference between the skill's Technology Selection Matrix and the
/// bundled catalogue's `matrix` rows, keyed by `requirement`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Drift {
  /// The skill lists a requirement row the catalogue has no matrix type for.
  RowAdded { requirement: String },
  /// The catalogue has a matrix type for a requirement the skill no longer lists.
  RowRemoved { requirement: String },
  /// The row exists in both, but one cell's text differs.
  CellChanged {
    requirement: String,
    column: &'static str,
    skill: String,
    catalog: String,
  },
}

impl fmt::Display for Drift {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Drift::RowAdded { requirement } => {
        write!(f, "row added in SKILL.md, missing from catalogue: {requirement}")
      }
      Drift::RowRemoved { requirement } => {
        write!(f, "row removed from SKILL.md, still in catalogue: {requirement}")
      }
      Drift::CellChanged {
        requirement,
        column,
        skill,
        catalog,
      } => write!(
        f,
        "{requirement}: {column} changed — catalogue has '{catalog}', SKILL.md has '{skill}'"
      ),
    }
  }
}

/// Parses the markdown table under the heading
/// "## Quick Reference: Technology Selection Matrix" (columns Requirement |
/// Recommended | Alternative | Avoid) and diffs it against `catalog`'s
/// bundled matrix rows, by requirement (FR-22).
pub fn diff_against_skill(catalog: &Catalog, skill_md: &str) -> Result<Vec<Drift>, CatalogError> {
  let skill_rows = parse_skill_matrix(skill_md)?;
  let skill_by_req: BTreeMap<&str, &MatrixRow> =
    skill_rows.iter().map(|r| (r.requirement.as_str(), r)).collect();

  let catalog_rows: Vec<&MatrixRow> = catalog.types.iter().filter_map(|t| t.matrix.as_ref()).collect();
  let catalog_by_req: BTreeMap<&str, &MatrixRow> =
    catalog_rows.iter().map(|r| (r.requirement.as_str(), *r)).collect();

  let mut drifts = Vec::new();

  for (req, skill_row) in &skill_by_req {
    match catalog_by_req.get(req) {
      None => drifts.push(Drift::RowAdded {
        requirement: req.to_string(),
      }),
      Some(cat_row) => {
        if skill_row.recommended != cat_row.recommended {
          drifts.push(Drift::CellChanged {
            requirement: req.to_string(),
            column: "Recommended",
            skill: skill_row.recommended.clone(),
            catalog: cat_row.recommended.clone(),
          });
        }
        if skill_row.alternative != cat_row.alternative {
          drifts.push(Drift::CellChanged {
            requirement: req.to_string(),
            column: "Alternative",
            skill: skill_row.alternative.clone(),
            catalog: cat_row.alternative.clone(),
          });
        }
        if skill_row.avoid != cat_row.avoid {
          drifts.push(Drift::CellChanged {
            requirement: req.to_string(),
            column: "Avoid",
            skill: skill_row.avoid.clone(),
            catalog: cat_row.avoid.clone(),
          });
        }
      }
    }
  }

  for req in catalog_by_req.keys() {
    if !skill_by_req.contains_key(req) {
      drifts.push(Drift::RowRemoved {
        requirement: req.to_string(),
      });
    }
  }

  Ok(drifts)
}

/// Parses the "Quick Reference: Technology Selection Matrix" markdown table
/// out of a `SKILL.md`'s text.
fn parse_skill_matrix(skill_md: &str) -> Result<Vec<MatrixRow>, CatalogError> {
  let mut lines = skill_md.lines();
  let mut found_heading = false;
  for line in lines.by_ref() {
    if line.trim() == SKILL_MATRIX_HEADING {
      found_heading = true;
      break;
    }
  }
  if !found_heading {
    return Err(CatalogError::SkillHeadingNotFound);
  }

  let mut table_lines: Vec<&str> = Vec::new();
  for line in lines {
    let trimmed = line.trim();
    if trimmed.starts_with('|') {
      table_lines.push(trimmed);
    } else if table_lines.is_empty() {
      // Blank lines (or other prose) before the table starts are fine.
      continue;
    } else {
      // The table ended.
      break;
    }
  }

  // table_lines[0] is the header row, table_lines[1] is the "---" separator.
  if table_lines.len() < 2 {
    return Err(CatalogError::SkillTableNotFound);
  }

  let mut rows = Vec::new();
  for line in table_lines.iter().skip(2) {
    let cells = parse_row_cells(line);
    if cells.len() != 4 {
      return Err(CatalogError::SkillTableRowShape {
        count: cells.len(),
        line: (*line).to_string(),
      });
    }
    rows.push(MatrixRow {
      requirement: cells[0].clone(),
      recommended: cells[1].clone(),
      alternative: cells[2].clone(),
      avoid: cells[3].clone(),
    });
  }
  Ok(rows)
}

fn parse_row_cells(line: &str) -> Vec<String> {
  let trimmed = line.trim();
  let inner = trimmed.trim_start_matches('|').trim_end_matches('|');
  inner.split('|').map(|c| c.trim().to_string()).collect()
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::model::catalog::OptionDef;

  fn types_yaml_fixture() -> &'static str {
    r#"
version: 1
types:
  - id: cloud-platform
    name: Cloud platform
    source: "Cloud Platform table"
    question: "Which cloud platform should host this project?"
    options:
      - id: azure
        name: Microsoft Azure
        ring: adopt
        description: "Primary."
        aliases: [azure]
      - id: hetzner
        name: Hetzner Cloud
        ring: trial
        description: "Budget."
        aliases: [hetzner]
      - id: hybrid
        name: Azure + Hetzner (hybrid)
        ring: trial
        description: "Hybrid."
        aliases: [hybrid]
  - id: rest-api-dotnet
    name: REST API (.NET)
    group: rest-api
    matrix: { requirement: "REST API (.NET)", recommended: ".NET 8 Minimal API", alternative: "ASP.NET Controllers", avoid: "WCF, .NET Framework" }
    question: "Which .NET API style?"
    options:
      - { id: dotnet-minimal-api, name: ".NET 8 Minimal API", ring: adopt, description: "x", aliases: [minimal api] }
      - { id: aspnet-controllers, name: "ASP.NET Controllers", ring: trial, description: "x", aliases: [controllers] }
      - { id: wcf, name: "WCF", ring: hold, description: "x", aliases: [wcf] }
      - { id: dotnet-framework, name: ".NET Framework", ring: hold, description: "x", aliases: [net framework] }
"#
  }

  fn criteria_yaml_fixture() -> &'static str {
    r#"
version: 1
criteria:
  - id: nfr_fit
    name: NFR fit
    weight: 0.25
    instructions: "x"
    levels: ["l0", "l1", "l2", "l3", "l4"]
"#
  }

  fn rules_yaml_fixture() -> &'static str {
    r#"
version: 1
groups:
  - group: rest-api
    by: backend-platform
    select:
      dotnet: [rest-api-dotnet]
auth_app_types:
  internal_enterprise: "x"
"#
  }

  fn minimal_catalog() -> Catalog {
    Catalog::from_strs(
      types_yaml_fixture(),
      criteria_yaml_fixture(),
      rules_yaml_fixture(),
      "template",
    )
    .expect("minimal fixture catalogue should be valid")
  }

  #[test]
  fn bundled_catalog_loads_and_has_27_types() {
    let catalog = Catalog::bundled().expect("bundled catalogue should load and validate");
    assert_eq!(catalog.types.len(), 27);
    assert!(catalog.type_by_id("cloud-platform").is_some());
    assert!(catalog.type_by_id("backend-platform").is_some());
    let matrix_types = catalog.types.iter().filter(|t| t.matrix.is_some()).count();
    assert_eq!(matrix_types, 25);
  }

  #[test]
  fn bundled_catalog_has_six_criteria_with_five_levels_each() {
    let catalog = Catalog::bundled().expect("bundled catalogue should load and validate");
    assert_eq!(catalog.criteria.len(), 6);
    for c in &catalog.criteria {
      assert_eq!(c.levels.len(), 5, "criterion {} should have 5 levels", c.id);
      assert!(c.weight > 0.0);
    }
  }

  #[test]
  fn rejects_duplicate_type_ids() {
    let types_yaml = format!("{}{}", types_yaml_fixture(), "  - id: cloud-platform\n    name: Dup\n    question: q\n    options: [{ id: a, name: A, ring: adopt, description: d }]\n");
    let err = Catalog::from_strs(&types_yaml, criteria_yaml_fixture(), rules_yaml_fixture(), "t").unwrap_err();
    assert!(matches!(err, CatalogError::DuplicateTypeId(id) if id == "cloud-platform"));
  }

  #[test]
  fn rejects_a_four_level_criterion() {
    let bad_criteria = r#"
version: 1
criteria:
  - id: nfr_fit
    name: NFR fit
    weight: 0.25
    instructions: "x"
    levels: ["l0", "l1", "l2", "l3"]
"#;
    let err = Catalog::from_strs(types_yaml_fixture(), bad_criteria, rules_yaml_fixture(), "t").unwrap_err();
    assert!(matches!(
      err,
      CatalogError::CriterionLevelCount { id, count } if id == "nfr_fit" && count == 4
    ));
  }

  #[test]
  fn rejects_a_rules_reference_to_a_missing_type() {
    let bad_rules = r#"
version: 1
groups:
  - group: rest-api
    by: backend-platform
    select:
      dotnet: [does-not-exist]
auth_app_types:
  internal_enterprise: "x"
"#;
    let err = Catalog::from_strs(types_yaml_fixture(), criteria_yaml_fixture(), bad_rules, "t").unwrap_err();
    assert!(matches!(
      err,
      CatalogError::UnknownRuleType { type_id, .. } if type_id == "does-not-exist"
    ));
  }

  #[test]
  fn rejects_a_ring_mismatch_against_the_matrix() {
    // Rename the "WCF" hold option without touching its ring: it stays hold,
    // but the "Avoid" cell's split text ("WCF, .NET Framework") no longer
    // names an option in the hold ring.
    let bad_types = types_yaml_fixture().replace(
      r#"{ id: wcf, name: "WCF", ring: hold, description: "x", aliases: [wcf] }"#,
      r#"{ id: wcf, name: "Not WCF Anymore", ring: hold, description: "x", aliases: [wcf] }"#,
    );
    let err = Catalog::from_strs(&bad_types, criteria_yaml_fixture(), rules_yaml_fixture(), "t").unwrap_err();
    assert!(matches!(
      err,
      CatalogError::MatrixMismatch { type_id, ring: Ring::Hold, .. } if type_id == "rest-api-dotnet"
    ));
  }

  #[test]
  fn rejects_zero_options() {
    let bad_types = r#"
version: 1
types:
  - id: empty-type
    name: Empty
    question: q
    options: []
"#;
    let err = Catalog::from_strs(bad_types, criteria_yaml_fixture(), "version: 1\ngroups: []\nauth_app_types: {}", "t")
      .unwrap_err();
    assert!(matches!(err, CatalogError::OptionCountOutOfRange { count: 0, .. }));
  }

  #[test]
  fn matrix_cells_with_slash_and_comma_split_into_separate_options() {
    let catalog = Catalog::bundled().expect("bundled catalogue should load and validate");
    let real_time = catalog.type_by_id("real-time").expect("real-time type should exist");
    let adopt_names: BTreeSet<String> = real_time
      .options
      .iter()
      .filter(|o| o.ring == Ring::Adopt)
      .map(|o| o.name.clone())
      .collect();
    assert_eq!(
      adopt_names,
      BTreeSet::from(["SignalR (.NET)".to_string(), "Socket.IO (Node)".to_string()])
    );

    let state_mgmt = catalog
      .type_by_id("state-management")
      .expect("state-management type should exist");
    let hold_names: BTreeSet<String> = state_mgmt
      .options
      .iter()
      .filter(|o| o.ring == Ring::Hold)
      .map(|o| o.name.clone())
      .collect();
    assert_eq!(hold_names, BTreeSet::from(["MobX".to_string(), "Recoil".to_string()]));
  }

  #[test]
  fn dash_cell_contributes_no_option() {
    let catalog = Catalog::bundled().expect("bundled catalogue should load and validate");
    let testing_node = catalog
      .type_by_id("testing-node")
      .expect("testing-node type should exist");
    let trial_count = testing_node.options.iter().filter(|o| o.ring == Ring::Trial).count();
    assert_eq!(trial_count, 0, "an '—' alternative cell should produce no trial option");
  }

  fn opt(id: &str, name: &str, aliases: &[&str]) -> OptionDef {
    OptionDef {
      id: id.to_string(),
      name: name.to_string(),
      ring: Ring::Hold,
      description: String::new(),
      aliases: aliases.iter().map(|s| s.to_string()).collect(),
    }
  }

  #[test]
  fn alias_matches_as_a_whole_word_case_insensitively() {
    let kafka = opt("kafka", "Kafka (unless scale demands)", &["kafka", "event hubs kafka"]);
    assert!(option_mentioned(&kafka, &["We already run Kafka".to_string()]));
    assert!(option_mentioned(&kafka, &["we use event hubs kafka today".to_string()]));
    assert!(!option_mentioned(&kafka, &["we use rabbitmq".to_string()]));
  }

  #[test]
  fn alias_matching_uses_word_boundaries_not_substrings() {
    // Documented rule: word-boundary matching, so a short alias like "node"
    // does not match inside a longer run-together token like "nodejs".
    let node = opt("node", "Node.js / TypeScript", &["node"]);
    assert!(!option_mentioned(&node, &["our stack is nodejs".to_string()]));
    assert!(option_mentioned(&node, &["our stack is node".to_string()]));
  }

  #[test]
  fn dotted_alias_matches_as_its_own_token() {
    let dotnet = opt("dotnet", ".NET 8+ / C#", &[".net", "dotnet"]);
    assert!(option_mentioned(&dotnet, &["we build everything in .net".to_string()]));
    assert!(option_mentioned(&dotnet, &["migrating off dotnet soon".to_string()]));

    let node_js = opt("node-js-alias", "Node.js", &["node.js"]);
    assert!(option_mentioned(&node_js, &["built with node.js".to_string()]));
  }

  #[test]
  fn diff_against_skill_reports_no_drift_for_matching_matrix() {
    let catalog = minimal_catalog();
    let skill_md = "## Quick Reference: Technology Selection Matrix\n\n| Requirement | Recommended | Alternative | Avoid |\n|---|---|---|---|\n| REST API (.NET) | .NET 8 Minimal API | ASP.NET Controllers | WCF, .NET Framework |\n";
    let drifts = diff_against_skill(&catalog, skill_md).expect("skill matrix should parse");
    assert!(drifts.is_empty());
  }

  #[test]
  fn diff_against_skill_reports_a_changed_cell() {
    let catalog = minimal_catalog();
    let skill_md = "## Quick Reference: Technology Selection Matrix\n\n| Requirement | Recommended | Alternative | Avoid |\n|---|---|---|---|\n| REST API (.NET) | .NET 8 Minimal API | ASP.NET Controllers | WCF |\n";
    let drifts = diff_against_skill(&catalog, skill_md).expect("skill matrix should parse");
    assert_eq!(drifts.len(), 1);
    assert!(matches!(&drifts[0], Drift::CellChanged { column, .. } if *column == "Avoid"));
  }

  #[test]
  fn diff_against_skill_reports_a_removed_row() {
    let catalog = minimal_catalog();
    let skill_md = "## Quick Reference: Technology Selection Matrix\n\n| Requirement | Recommended | Alternative | Avoid |\n|---|---|---|---|\n";
    let drifts = diff_against_skill(&catalog, skill_md).expect("skill matrix should parse");
    assert_eq!(drifts, vec![Drift::RowRemoved { requirement: "REST API (.NET)".to_string() }]);
  }

  #[test]
  fn diff_against_skill_reports_an_added_row() {
    let catalog = minimal_catalog();
    let skill_md = "## Quick Reference: Technology Selection Matrix\n\n| Requirement | Recommended | Alternative | Avoid |\n|---|---|---|---|\n| REST API (.NET) | .NET 8 Minimal API | ASP.NET Controllers | WCF, .NET Framework |\n| New Row | X | Y | Z |\n";
    let drifts = diff_against_skill(&catalog, skill_md).expect("skill matrix should parse");
    assert_eq!(drifts, vec![Drift::RowAdded { requirement: "New Row".to_string() }]);
  }
}
