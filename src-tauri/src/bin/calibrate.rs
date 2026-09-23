//! `cargo run --bin calibrate` (spec FR-17, AC-19): validates the golden set
//! offline (`--dry-run`), or runs it against the live OpenRouter Decisions
//! API and reports accuracy across a threshold/margin grid.
//!
//! `--dry-run` loads every `golden/*.yaml`, deserializes it as a `GoldenCase`,
//! and validates it (brief passes `brief::validate_brief`, every expected
//! type/option id exists in the bundled catalogue, ids are unique, and there
//! are at least `MIN_CASES`). It makes no network calls. Live mode (the
//! default) additionally requires `OPENROUTER_API_KEY`, runs each case
//! through the real pipeline with a `FakeLocalModel` that hands back the
//! case's own Brief (so the pipeline path is exercised exactly as in the
//! app) and a real `HttpJev`, then re-routes the raw composite rankings
//! offline across a grid of thresholds and margins (no extra Jev calls) to
//! report calibration metrics.

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use serde::Deserialize;

use bistec_architect::brief::validate_brief;
use bistec_architect::jev::HttpJev;
use bistec_architect::model::brief::Brief;
use bistec_architect::model::catalog::{Catalog, Ring};
use bistec_architect::model::decision::{OptionScore, ReasonCode, Route};
use bistec_architect::model::settings::Settings;
use bistec_architect::ollama::FakeLocalModel;
use bistec_architect::pipeline::{build_report, confirm_brief, run_decisions, start_describe, Deps, NoopSink, MIN_DESCRIBE_CHARS};
use bistec_architect::scoring::{route, RoutingConfig};
use bistec_architect::store::Store;

/// A "route every decision to this value" catch-all key for `expected_route`
/// (used by the prompt-injection golden case).
const EXPECTED_ROUTE_ANY: &str = "*";
/// AC-19: `golden/` must contain at least this many cases.
const MIN_CASES: usize = 10;
/// The threshold/margin grid the live report sweeps (design FR-17).
const THRESHOLDS: [f64; 5] = [0.3, 0.4, 0.5, 0.6, 0.7];
const MIN_MARGINS: [f64; 3] = [0.0, 0.05, 0.1];

// ---------------------------------------------------------------------
// Golden case format
// ---------------------------------------------------------------------

/// One `golden/NN-<slug>.yaml` case.
#[derive(Debug, Deserialize)]
struct GoldenCase {
  id: String,
  #[serde(default)]
  description: String,
  brief: Brief,
  /// `type_id -> expected option_id`, for the types this case is about.
  #[serde(default)]
  expected: BTreeMap<String, String>,
  /// `type_id (or "*" for every decision) -> "proposed" | "needs_architect"`.
  #[serde(default)]
  expected_route: BTreeMap<String, String>,
  /// `"out_of_remit" | "done"`.
  #[serde(default)]
  expected_stage: Option<String>,
  #[serde(default)]
  rationale: String,
}

/// Loads every `*.yaml`/`*.yml` file directly under `dir`, in filename order.
/// A file that fails to read or parse is reported as an error string rather
/// than aborting the whole load, so `--dry-run` can report every problem in
/// one pass.
fn load_cases(dir: &Path) -> Result<(Vec<GoldenCase>, Vec<String>), String> {
  let mut paths: Vec<PathBuf> = fs::read_dir(dir)
    .map_err(|e| format!("failed to read golden directory {}: {e}", dir.display()))?
    .filter_map(|entry| entry.ok().map(|e| e.path()))
    .filter(|p| matches!(p.extension().and_then(|e| e.to_str()), Some("yaml") | Some("yml")))
    .collect();
  paths.sort();

  let mut cases = Vec::with_capacity(paths.len());
  let mut errors = Vec::new();
  for path in paths {
    match fs::read_to_string(&path) {
      Ok(text) => match serde_yaml::from_str::<GoldenCase>(&text) {
        Ok(case) => cases.push(case),
        Err(e) => errors.push(format!("{}: failed to parse: {e}", path.display())),
      },
      Err(e) => errors.push(format!("{}: failed to read: {e}", path.display())),
    }
  }
  Ok((cases, errors))
}

/// Validates the loaded golden set (AC-19): at least `MIN_CASES`, unique
/// ids, every brief passes `validate_brief`, and every `expected` /
/// `expected_route` type (and `expected` option) id exists in `catalog`.
fn validate_cases(cases: &[GoldenCase], catalog: &Catalog) -> Vec<String> {
  let mut errors = Vec::new();
  if cases.len() < MIN_CASES {
    errors.push(format!("expected at least {MIN_CASES} golden cases, found {}", cases.len()));
  }

  let mut seen_ids = BTreeSet::new();
  for case in cases {
    if !seen_ids.insert(case.id.as_str()) {
      errors.push(format!("duplicate case id: {}", case.id));
    }
    if let Err(e) = validate_brief(&case.brief) {
      errors.push(format!("{}: invalid brief: {e}", case.id));
    }

    for (type_id, option_id) in &case.expected {
      match catalog.type_by_id(type_id) {
        None => errors.push(format!("{}: expected type '{type_id}' is not in the catalogue", case.id)),
        Some(dt) => {
          if !dt.options.iter().any(|o| &o.id == option_id) {
            errors.push(format!("{}: expected option '{option_id}' is not in type '{type_id}'", case.id));
          }
        }
      }
    }

    for (type_id, r) in &case.expected_route {
      if type_id != EXPECTED_ROUTE_ANY && catalog.type_by_id(type_id).is_none() {
        errors.push(format!("{}: expected_route type '{type_id}' is not in the catalogue", case.id));
      }
      if !matches!(r.as_str(), "proposed" | "needs_architect") {
        errors.push(format!(
          "{}: expected_route['{type_id}'] must be 'proposed' or 'needs_architect', got '{r}'",
          case.id
        ));
      }
    }

    if let Some(stage) = &case.expected_stage {
      if !matches!(stage.as_str(), "out_of_remit" | "done") {
        errors.push(format!("{}: expected_stage must be 'out_of_remit' or 'done', got '{stage}'", case.id));
      }
    }

    if case.expected.is_empty() && case.expected_route.is_empty() && case.expected_stage.is_none() {
      errors.push(format!(
        "{}: declares no expected outcome at all (expected / expected_route / expected_stage)",
        case.id
      ));
    }
  }

  errors
}

fn print_case_summary(cases: &[GoldenCase]) {
  println!("{:<40} {:>8} {:>10} {:>14}  description", "id", "expected", "exp_route", "exp_stage");
  for case in cases {
    println!(
      "{:<40} {:>8} {:>10} {:>14}  {}",
      case.id,
      case.expected.len(),
      case.expected_route.len(),
      case.expected_stage.as_deref().unwrap_or("-"),
      case.description,
    );
  }
}

// ---------------------------------------------------------------------
// CLI args
// ---------------------------------------------------------------------

struct Args {
  dry_run: bool,
  golden_dir: PathBuf,
  case_filter: Option<String>,
  max_cost: f64,
}

/// `golden/` relative to the repo root, resolved at compile time from
/// `CARGO_MANIFEST_DIR` (this crate lives in `src-tauri/`) so the default
/// works regardless of the process's current directory.
fn default_golden_dir() -> PathBuf {
  Path::new(env!("CARGO_MANIFEST_DIR"))
    .parent()
    .expect("CARGO_MANIFEST_DIR should have a parent directory (the repo root)")
    .join("golden")
}

fn parse_args(args: &[String]) -> Result<Args, String> {
  let mut dry_run = false;
  let mut golden_dir = default_golden_dir();
  let mut case_filter = None;
  let mut max_cost = 1.0;

  let mut i = 0;
  while i < args.len() {
    match args[i].as_str() {
      "--dry-run" => dry_run = true,
      "--golden" => {
        i += 1;
        golden_dir = PathBuf::from(args.get(i).ok_or("--golden requires a directory path")?);
      }
      "--case" => {
        i += 1;
        case_filter = Some(args.get(i).ok_or("--case requires a case id")?.clone());
      }
      "--max-cost" => {
        i += 1;
        let v = args.get(i).ok_or("--max-cost requires a dollar amount")?;
        max_cost = v.parse().map_err(|_| format!("--max-cost: '{v}' is not a number"))?;
      }
      other => return Err(format!("unknown argument: {other}")),
    }
    i += 1;
  }

  Ok(Args {
    dry_run,
    golden_dir,
    case_filter,
    max_cost,
  })
}

// ---------------------------------------------------------------------
// Live run: one case
// ---------------------------------------------------------------------

/// One decision's recorded outcome, enough to recompute its route offline
/// under a different threshold/margin (spec `scoring::route`'s inputs).
#[derive(Debug, Clone)]
struct DecisionRecord {
  type_id: String,
  choice: String,
  confidence: f64,
  ring: Ring,
  scores: Vec<OptionScore>,
  injection: bool,
  /// The expected option id for this type, if this case declared one.
  expected: Option<String>,
}

/// One case's live-run outcome.
struct CaseRun {
  id: String,
  /// The session's final stage (`"done"`, `"out_of_remit"`, or `"failed:*"`).
  stage: String,
  expected_stage: Option<String>,
  expected_route: BTreeMap<String, String>,
  /// The raw `expected` map, so unresolved (never-decided) expectations can
  /// still be counted as a diagnostic, separately from routing accuracy.
  expected: BTreeMap<String, String>,
  decisions: Vec<DecisionRecord>,
  cost_usd: Option<f64>,
  input_tokens: u64,
}

/// Mode A needs at least `MIN_DESCRIBE_CHARS`; the exact text is otherwise
/// irrelevant here since `FakeLocalModel` ignores it and replays the case's
/// own Brief regardless.
fn describe_text(case: &GoldenCase) -> String {
  let text = format!("{} — {}", case.description, case.brief.summary);
  if text.chars().count() >= MIN_DESCRIBE_CHARS {
    text
  } else {
    format!("{text} {}", case.rationale)
  }
}

async fn run_one_case(catalog: &Arc<Catalog>, case: &GoldenCase, api_key: &str) -> Result<CaseRun, String> {
  let store = Arc::new(Store::open_in_memory().map_err(|e| e.to_string())?);
  store.ack_data_notice().map_err(|e| e.to_string())?;
  store.save_settings(&Settings::default()).map_err(|e| e.to_string())?;

  let brief_json = serde_json::to_string(&case.brief).map_err(|e| e.to_string())?;
  let deps = Deps {
    catalog: catalog.clone(),
    store: store.clone(),
    jev: Arc::new(HttpJev::new(api_key.to_string())),
    local: Arc::new(FakeLocalModel::new("golden", true, vec![Ok(brief_json)])),
  };

  let text = describe_text(case);
  let session_id = start_describe(&deps, &text).await.map_err(|e| e.to_string())?;
  confirm_brief(&deps, &session_id).map_err(|e| e.to_string())?;
  run_decisions(&deps, &session_id, &NoopSink).await.map_err(|e| e.to_string())?;

  let row = store.get_session(&session_id).ok_or("session vanished after run_decisions")?;
  let report = build_report(&deps, &session_id).map_err(|e| e.to_string())?;

  let mut decisions = Vec::new();
  if let Some(report) = &report {
    for view in &report.decisions {
      let d = &view.decision;
      let ring = catalog
        .type_by_id(&d.type_id)
        .and_then(|dt| dt.options.iter().find(|o| o.id == d.choice))
        .map(|o| o.ring)
        .ok_or_else(|| format!("decision '{}' chose an unknown option '{}'", d.type_id, d.choice))?;
      decisions.push(DecisionRecord {
        type_id: d.type_id.clone(),
        choice: d.choice.clone(),
        confidence: d.confidence,
        ring,
        scores: d.option_scores.clone(),
        injection: d.reasons.contains(&ReasonCode::PossibleInjection),
        expected: case.expected.get(&d.type_id).cloned(),
      });
    }
  }

  Ok(CaseRun {
    id: case.id.clone(),
    stage: row.stage,
    expected_stage: case.expected_stage.clone(),
    expected_route: case.expected_route.clone(),
    expected: case.expected.clone(),
    decisions,
    cost_usd: report.as_ref().and_then(|r| r.cost_usd),
    input_tokens: report.as_ref().map(|r| r.input_tokens).unwrap_or(0),
  })
}

// ---------------------------------------------------------------------
// Offline evaluation (pure; unit-tested with synthetic CaseRuns below)
// ---------------------------------------------------------------------

fn route_to_str(r: Route) -> &'static str {
  match r {
    Route::Proposed => "proposed",
    Route::NeedsArchitect => "needs_architect",
  }
}

fn ratio(n: usize, d: usize) -> f64 {
  if d == 0 {
    0.0
  } else {
    n as f64 / d as f64
  }
}

/// Re-derives a decision's route under `cfg` from its already-recorded
/// composite ranking (spec `scoring::route`): no extra Jev calls.
fn recompute_route(cfg: &RoutingConfig, rec: &DecisionRecord) -> (Route, Vec<ReasonCode>) {
  route(&rec.choice, rec.confidence, rec.ring, &rec.scores, rec.injection, cfg)
}

/// One grid point's aggregate metrics across every case's decisions.
#[derive(Debug, Clone, PartialEq)]
struct GridResult {
  threshold: f64,
  min_margin: f64,
  /// `choice == expected` over decisions with a declared `expected` option.
  accuracy: f64,
  expected_n: usize,
  /// `choice == expected` over decisions with a declared `expected` option
  /// that also route Proposed at this grid point.
  precision_proposed: f64,
  proposed_n: usize,
  /// Share of every decision (not just ones with an `expected` option) that
  /// routes Needs architect at this grid point.
  share_needs_architect: f64,
  total_decisions: usize,
  /// Agreement between the recomputed route and a case's `expected_route`.
  route_agreement: f64,
  route_n: usize,
}

fn evaluate_grid_point(cases: &[CaseRun], cfg: &RoutingConfig) -> GridResult {
  let mut expected_correct = 0usize;
  let mut expected_total = 0usize;
  let mut proposed_correct = 0usize;
  let mut proposed_total = 0usize;
  let mut needs_architect = 0usize;
  let mut all_decisions = 0usize;
  let mut route_correct = 0usize;
  let mut route_total = 0usize;

  for case in cases {
    for dec in &case.decisions {
      let (r, _reasons) = recompute_route(cfg, dec);
      all_decisions += 1;
      if r == Route::NeedsArchitect {
        needs_architect += 1;
      }

      if let Some(expected_option) = &dec.expected {
        let is_correct = &dec.choice == expected_option;
        expected_total += 1;
        expected_correct += is_correct as usize;
        if r == Route::Proposed {
          proposed_total += 1;
          proposed_correct += is_correct as usize;
        }
      }

      if let Some(expected_route_str) = case
        .expected_route
        .get(&dec.type_id)
        .or_else(|| case.expected_route.get(EXPECTED_ROUTE_ANY))
      {
        route_total += 1;
        if route_to_str(r) == expected_route_str {
          route_correct += 1;
        }
      }
    }
  }

  GridResult {
    threshold: cfg.threshold,
    min_margin: cfg.min_margin,
    accuracy: ratio(expected_correct, expected_total),
    expected_n: expected_total,
    precision_proposed: ratio(proposed_correct, proposed_total),
    proposed_n: proposed_total,
    share_needs_architect: ratio(needs_architect, all_decisions),
    total_decisions: all_decisions,
    route_agreement: ratio(route_correct, route_total),
    route_n: route_total,
  }
}

fn evaluate_grid(cases: &[CaseRun]) -> Vec<GridResult> {
  let mut out = Vec::with_capacity(THRESHOLDS.len() * MIN_MARGINS.len());
  for &threshold in &THRESHOLDS {
    for &min_margin in &MIN_MARGINS {
      let cfg = RoutingConfig {
        threshold,
        min_margin,
        weights: BTreeMap::new(),
      };
      out.push(evaluate_grid_point(cases, &cfg));
    }
  }
  out
}

/// `expected_stage` agreement, independent of the threshold grid (the gate
/// stage that decides `out_of_remit` vs `done` doesn't depend on
/// `threshold`/`min_margin` at all).
fn stage_agreement(cases: &[CaseRun]) -> (usize, usize) {
  let mut correct = 0;
  let mut total = 0;
  for case in cases {
    if let Some(expected) = &case.expected_stage {
      total += 1;
      if &case.stage == expected {
        correct += 1;
      }
    }
  }
  (correct, total)
}

/// How many `expected` entries never resolved to an actual decision (the
/// type was skipped or judged not applicable) — a distinct, applicability-
/// calibration failure mode from a wrong-choice or bad-routing one.
fn missing_expected(cases: &[CaseRun]) -> usize {
  let mut missing = 0;
  for case in cases {
    let decided: BTreeSet<&str> = case.decisions.iter().map(|d| d.type_id.as_str()).collect();
    missing += case.expected.keys().filter(|t| !decided.contains(t.as_str())).count();
  }
  missing
}

fn render_calibration_report(cases: &[CaseRun], total_cost: f64) -> String {
  let grid = evaluate_grid(cases);
  let (stage_correct, stage_total) = stage_agreement(cases);
  let missing = missing_expected(cases);
  let total_input_tokens: u64 = cases.iter().map(|c| c.input_tokens).sum();

  let mut out = String::new();
  out.push_str("# Golden-set calibration report\n\n");
  out.push_str(&format!("- Cases run: {}\n", cases.len()));
  out.push_str(&format!("- Total cost: ${total_cost:.4}\n"));
  out.push_str(&format!("- Total input tokens: {total_input_tokens}\n"));
  out.push_str(&format!("- Expected-stage agreement: {stage_correct}/{stage_total}\n"));
  out.push_str(&format!(
    "- Expected options never decided (not applicable/skipped): {missing}\n\n"
  ));

  out.push_str(
    "| threshold | min_margin | accuracy | n | precision(Proposed) | n | share needs_architect | n | expected_route agreement | n |\n",
  );
  out.push_str("|---|---|---|---|---|---|---|---|---|---|\n");
  for g in &grid {
    out.push_str(&format!(
      "| {:.2} | {:.2} | {:.3} | {} | {:.3} | {} | {:.3} | {} | {:.3} | {} |\n",
      g.threshold,
      g.min_margin,
      g.accuracy,
      g.expected_n,
      g.precision_proposed,
      g.proposed_n,
      g.share_needs_architect,
      g.total_decisions,
      g.route_agreement,
      g.route_n,
    ));
  }
  out
}

// ---------------------------------------------------------------------
// main
// ---------------------------------------------------------------------

async fn run_live(catalog: Arc<Catalog>, cases: &[GoldenCase], api_key: &str, args: &Args) -> ExitCode {
  let selected: Vec<&GoldenCase> = cases
    .iter()
    .filter(|c| args.case_filter.as_deref().map_or(true, |f| f == c.id))
    .collect();
  if selected.is_empty() {
    eprintln!(
      "calibrate: no case matches --case '{}'",
      args.case_filter.as_deref().unwrap_or("")
    );
    return ExitCode::from(1);
  }

  let mut runs = Vec::new();
  let mut total_cost = 0.0_f64;

  for case in selected {
    if total_cost > args.max_cost {
      println!(
        "calibrate: cumulative cost ${total_cost:.4} exceeds --max-cost ${:.4}; stopping before case '{}'",
        args.max_cost, case.id
      );
      break;
    }

    println!("calibrate: running case '{}' ({})", case.id, case.description);
    match run_one_case(&catalog, case, api_key).await {
      Ok(run) => {
        println!(
          "calibrate: case '{}' finished (stage={}, decisions={}, cost=${:.4})",
          run.id,
          run.stage,
          run.decisions.len(),
          run.cost_usd.unwrap_or(0.0)
        );
        total_cost += run.cost_usd.unwrap_or(0.0);
        runs.push(run);
      }
      Err(e) => eprintln!("calibrate: case '{}' failed: {e}", case.id),
    }
  }

  if runs.is_empty() {
    eprintln!("calibrate: no case completed successfully");
    return ExitCode::from(1);
  }

  let report = render_calibration_report(&runs, total_cost);
  println!("\n{report}");

  let report_path = args.golden_dir.join("calibration-report.md");
  if let Err(e) = fs::write(&report_path, report.as_bytes()) {
    eprintln!("calibrate: failed to write {}: {e}", report_path.display());
    return ExitCode::from(1);
  }
  println!("calibrate: wrote {}", report_path.display());

  ExitCode::SUCCESS
}

#[tokio::main]
async fn main() -> ExitCode {
  let raw_args: Vec<String> = env::args().skip(1).collect();
  let args = match parse_args(&raw_args) {
    Ok(a) => a,
    Err(e) => {
      eprintln!("calibrate: {e}");
      return ExitCode::from(2);
    }
  };

  let catalog = match Catalog::bundled() {
    Ok(c) => c,
    Err(e) => {
      eprintln!("calibrate: failed to load the bundled catalogue: {e}");
      return ExitCode::from(2);
    }
  };

  let (cases, mut errors) = match load_cases(&args.golden_dir) {
    Ok(v) => v,
    Err(e) => {
      eprintln!("calibrate: {e}");
      return ExitCode::from(2);
    }
  };

  print_case_summary(&cases);
  errors.extend(validate_cases(&cases, &catalog));

  if !errors.is_empty() {
    println!("\n{} validation error(s):", errors.len());
    for e in &errors {
      println!("  - {e}");
    }
    return ExitCode::from(1);
  }
  println!("\n{} golden case(s) valid.", cases.len());

  if args.dry_run {
    return ExitCode::SUCCESS;
  }

  let api_key = match env::var("OPENROUTER_API_KEY") {
    Ok(k) if !k.trim().is_empty() => k,
    _ => {
      eprintln!(
        "calibrate: OPENROUTER_API_KEY is not set. Live calibration needs a real OpenRouter API key \
         with credit; pass --dry-run to validate the golden set offline instead."
      );
      return ExitCode::from(2);
    }
  };

  run_live(Arc::new(catalog), &cases, &api_key, &args).await
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
  use super::*;

  fn score(option_id: &str, composite: f64) -> OptionScore {
    OptionScore {
      option_id: option_id.to_string(),
      criterion_scores: BTreeMap::new(),
      composite,
    }
  }

  fn decision(type_id: &str, choice: &str, confidence: f64, ring: Ring, scores: Vec<OptionScore>, expected: Option<&str>) -> DecisionRecord {
    DecisionRecord {
      type_id: type_id.to_string(),
      choice: choice.to_string(),
      confidence,
      ring,
      scores,
      injection: false,
      expected: expected.map(str::to_string),
    }
  }

  fn case_run(id: &str, decisions: Vec<DecisionRecord>) -> CaseRun {
    CaseRun {
      id: id.to_string(),
      stage: "done".to_string(),
      expected_stage: None,
      expected_route: BTreeMap::new(),
      expected: BTreeMap::new(),
      decisions,
      cost_usd: Some(0.01),
      input_tokens: 100,
    }
  }

  fn cfg(threshold: f64, min_margin: f64) -> RoutingConfig {
    RoutingConfig {
      threshold,
      min_margin,
      weights: BTreeMap::new(),
    }
  }

  // ---- validate_cases -----------------------------------------------

  fn minimal_brief() -> Brief {
    serde_json::from_value(serde_json::json!({
      "summary": "A tool.",
      "context": {
        "scale": "unknown", "budget": "unknown", "timeline": "unknown",
        "team_size": "unknown", "compliance": [], "data_sensitivity": "unknown"
      },
      "requirements": [], "nfrs": [], "constraints": [], "team_skills": [],
      "mentioned_technologies": []
    }))
    .unwrap()
  }

  fn case(id: &str) -> GoldenCase {
    GoldenCase {
      id: id.to_string(),
      description: "d".to_string(),
      brief: minimal_brief(),
      expected: BTreeMap::new(),
      expected_route: BTreeMap::new(),
      expected_stage: None,
      rationale: "r".to_string(),
    }
  }

  fn ten_cases() -> Vec<GoldenCase> {
    (0..10)
      .map(|i| {
        let mut c = case(&format!("case-{i}"));
        c.expected_stage = Some("done".to_string());
        c
      })
      .collect()
  }

  #[test]
  fn validate_cases_rejects_fewer_than_min_cases() {
    let cat = Catalog::bundled().unwrap();
    let errors = validate_cases(&ten_cases()[..9], &cat);
    assert!(errors.iter().any(|e| e.contains("at least 10")));
  }

  #[test]
  fn validate_cases_accepts_ten_valid_cases() {
    let cat = Catalog::bundled().unwrap();
    let errors = validate_cases(&ten_cases(), &cat);
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
  }

  #[test]
  fn validate_cases_rejects_duplicate_ids() {
    let cat = Catalog::bundled().unwrap();
    let mut cases = ten_cases();
    cases[1].id = cases[0].id.clone();
    let errors = validate_cases(&cases, &cat);
    assert!(errors.iter().any(|e| e.contains("duplicate case id")));
  }

  #[test]
  fn validate_cases_rejects_an_unknown_expected_type() {
    let cat = Catalog::bundled().unwrap();
    let mut cases = ten_cases();
    cases[0].expected.insert("no-such-type".to_string(), "azure".to_string());
    let errors = validate_cases(&cases, &cat);
    assert!(errors.iter().any(|e| e.contains("no-such-type") && e.contains("not in the catalogue")));
  }

  #[test]
  fn validate_cases_rejects_an_unknown_expected_option() {
    let cat = Catalog::bundled().unwrap();
    let mut cases = ten_cases();
    cases[0].expected.insert("cloud-platform".to_string(), "aws".to_string());
    let errors = validate_cases(&cases, &cat);
    assert!(errors.iter().any(|e| e.contains("'aws'") && e.contains("cloud-platform")));
  }

  #[test]
  fn validate_cases_accepts_the_route_any_catch_all_key() {
    let cat = Catalog::bundled().unwrap();
    let mut cases = ten_cases();
    cases[0].expected_stage = None;
    cases[0].expected_route.insert(EXPECTED_ROUTE_ANY.to_string(), "needs_architect".to_string());
    let errors = validate_cases(&cases, &cat);
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
  }

  #[test]
  fn validate_cases_rejects_an_invalid_expected_route_value() {
    let cat = Catalog::bundled().unwrap();
    let mut cases = ten_cases();
    cases[0].expected_route.insert("cloud-platform".to_string(), "maybe".to_string());
    let errors = validate_cases(&cases, &cat);
    assert!(errors.iter().any(|e| e.contains("must be 'proposed' or 'needs_architect'")));
  }

  #[test]
  fn validate_cases_rejects_an_invalid_expected_stage() {
    let cat = Catalog::bundled().unwrap();
    let mut cases = ten_cases();
    cases[0].expected_stage = Some("maybe".to_string());
    let errors = validate_cases(&cases, &cat);
    assert!(errors.iter().any(|e| e.contains("expected_stage must be")));
  }

  #[test]
  fn validate_cases_rejects_a_case_with_no_expected_outcome_declared() {
    let cat = Catalog::bundled().unwrap();
    let mut cases = ten_cases();
    cases[0].expected_stage = None;
    let errors = validate_cases(&cases, &cat);
    assert!(errors.iter().any(|e| e.contains("declares no expected outcome")));
  }

  #[test]
  fn validate_cases_rejects_an_invalid_brief() {
    let cat = Catalog::bundled().unwrap();
    let mut cases = ten_cases();
    cases[0].brief.summary = "   ".to_string();
    let errors = validate_cases(&cases, &cat);
    assert!(errors.iter().any(|e| e.contains("invalid brief")));
  }

  // ---- evaluate_grid_point / route recomputation ---------------------

  #[test]
  fn accuracy_and_precision_match_hand_computed_figures() {
    // Two decisions with a declared `expected`: one where the choice IS the
    // top composite (correct, and routes Proposed), one where the choice is
    // NOT the top composite (wrong, and routes Needs architect because of
    // the `disagreement` reason).
    let cases = vec![case_run(
      "c1",
      vec![
        decision("cloud-platform", "azure", 0.9, Ring::Adopt, vec![score("azure", 0.8), score("hetzner", 0.2)], Some("azure")),
        decision(
          "backend-platform",
          "node",
          0.9,
          Ring::Trial,
          vec![score("dotnet", 0.8), score("node", 0.2)],
          Some("dotnet"),
        ),
      ],
    )];
    let g = evaluate_grid_point(&cases, &cfg(0.5, 0.05));
    assert_eq!(g.expected_n, 2);
    assert!((g.accuracy - 0.5).abs() < 1e-9, "got {}", g.accuracy);
    // Only the "cloud-platform" decision routes Proposed; "backend-platform"
    // disagrees with its own top composite and routes Needs architect.
    assert_eq!(g.proposed_n, 1);
    assert!((g.precision_proposed - 1.0).abs() < 1e-9, "got {}", g.precision_proposed);
  }

  #[test]
  fn a_higher_threshold_routes_more_decisions_to_needs_architect() {
    let cases = vec![case_run(
      "c1",
      vec![decision(
        "cloud-platform",
        "azure",
        0.55,
        Ring::Adopt,
        vec![score("azure", 0.9), score("hetzner", 0.1)],
        Some("azure"),
      )],
    )];
    let lenient = evaluate_grid_point(&cases, &cfg(0.5, 0.0));
    let strict = evaluate_grid_point(&cases, &cfg(0.6, 0.0));
    assert_eq!(lenient.share_needs_architect, 0.0);
    assert_eq!(strict.share_needs_architect, 1.0);
  }

  #[test]
  fn a_hold_ring_choice_always_routes_needs_architect_regardless_of_threshold() {
    let cases = vec![case_run(
      "c1",
      vec![decision(
        "message-queue-simple",
        "kafka",
        0.99,
        Ring::Hold,
        vec![score("kafka", 0.9), score("azure-queue-storage", 0.1)],
        None,
      )],
    )];
    for &threshold in &THRESHOLDS {
      let g = evaluate_grid_point(&cases, &cfg(threshold, 0.0));
      assert_eq!(g.share_needs_architect, 1.0, "threshold {threshold}");
    }
  }

  #[test]
  fn expected_route_any_catch_all_applies_to_every_decision_in_the_case() {
    let mut c = case_run(
      "c1",
      vec![
        decision("cloud-platform", "azure", 0.1, Ring::Adopt, vec![score("azure", 0.9)], None),
        decision("backend-platform", "dotnet", 0.1, Ring::Adopt, vec![score("dotnet", 0.9)], None),
      ],
    );
    c.expected_route.insert(EXPECTED_ROUTE_ANY.to_string(), "needs_architect".to_string());
    let g = evaluate_grid_point(&[c], &cfg(0.5, 0.05));
    // low confidence (0.1) routes both decisions Needs architect.
    assert_eq!(g.route_n, 2);
    assert_eq!(g.route_agreement, 1.0);
  }

  #[test]
  fn expected_route_for_a_specific_type_overrides_the_catch_all() {
    let mut c = case_run(
      "c1",
      vec![decision("cloud-platform", "azure", 0.9, Ring::Adopt, vec![score("azure", 0.9), score("hetzner", 0.1)], None)],
    );
    c.expected_route.insert(EXPECTED_ROUTE_ANY.to_string(), "needs_architect".to_string());
    c.expected_route.insert("cloud-platform".to_string(), "proposed".to_string());
    let g = evaluate_grid_point(&[c], &cfg(0.5, 0.05));
    assert_eq!(g.route_n, 1);
    assert_eq!(g.route_agreement, 1.0, "the specific-type entry should win, not the catch-all");
  }

  #[test]
  fn missing_expected_counts_expectations_with_no_matching_decision() {
    let mut c = case_run(
      "c1",
      vec![decision("cloud-platform", "azure", 0.9, Ring::Adopt, vec![score("azure", 0.9)], Some("azure"))],
    );
    // "auth-enterprise" was expected but never decided (not applicable).
    c.expected.insert("cloud-platform".to_string(), "azure".to_string());
    c.expected.insert("auth-enterprise".to_string(), "entra-id".to_string());
    assert_eq!(missing_expected(&[c]), 1);
  }

  #[test]
  fn stage_agreement_counts_only_cases_with_an_expected_stage() {
    let mut done = case_run("c1", vec![]);
    done.stage = "done".to_string();
    done.expected_stage = Some("done".to_string());

    let mut wrong = case_run("c2", vec![]);
    wrong.stage = "out_of_remit".to_string();
    wrong.expected_stage = Some("done".to_string());

    let mut unset = case_run("c3", vec![]);
    unset.stage = "done".to_string();

    let (correct, total) = stage_agreement(&[done, wrong, unset]);
    assert_eq!((correct, total), (1, 2));
  }

  #[test]
  fn ratio_of_zero_denominator_is_zero_not_nan() {
    assert_eq!(ratio(0, 0), 0.0);
    assert_eq!(ratio(3, 6), 0.5);
  }

  #[test]
  fn render_calibration_report_contains_the_grid_header_and_totals() {
    let cases = vec![case_run(
      "c1",
      vec![decision("cloud-platform", "azure", 0.9, Ring::Adopt, vec![score("azure", 0.9)], Some("azure"))],
    )];
    let report = render_calibration_report(&cases, 0.05);
    assert!(report.contains("Cases run: 1"));
    assert!(report.contains("Total cost: $0.0500"));
    assert!(report.contains("| threshold | min_margin |"));
    // 5 thresholds x 3 margins = 15 grid rows.
    let grid_rows = report.lines().filter(|l| l.starts_with("| 0.")).count();
    assert_eq!(grid_rows, THRESHOLDS.len() * MIN_MARGINS.len());
  }

  // ---- CLI arg parsing -------------------------------------------------

  #[test]
  fn parse_args_defaults() {
    let args = parse_args(&[]).unwrap();
    assert!(!args.dry_run);
    assert!(args.case_filter.is_none());
    assert_eq!(args.max_cost, 1.0);
    assert!(args.golden_dir.ends_with("golden"));
  }

  #[test]
  fn parse_args_reads_every_flag() {
    let raw: Vec<String> = ["--dry-run", "--golden", "/tmp/g", "--case", "01-x", "--max-cost", "2.5"]
      .iter()
      .map(|s| s.to_string())
      .collect();
    let args = parse_args(&raw).unwrap();
    assert!(args.dry_run);
    assert_eq!(args.golden_dir, PathBuf::from("/tmp/g"));
    assert_eq!(args.case_filter.as_deref(), Some("01-x"));
    assert_eq!(args.max_cost, 2.5);
  }

  #[test]
  fn parse_args_rejects_an_unknown_flag() {
    assert!(parse_args(&["--bogus".to_string()]).is_err());
  }
}
