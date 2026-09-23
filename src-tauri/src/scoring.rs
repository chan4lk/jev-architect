//! Composition and routing (spec FR-14, design "Routing (scoring.rs, pure)").
//! Everything here is a pure function over already-received Jev answers:
//! no network calls, no randomness, and no wall-clock reads (NFR-5).

use std::collections::BTreeMap;

use thiserror::Error;

use crate::jev::Answer;
use crate::model::catalog::{Catalog, DecisionType, Ring};
use crate::model::decision::{OptionScore, ReasonCode, Route};

/// Gate answers at or above this Noul value count as "yes" (spec FR-10).
pub const GATE_THRESHOLD: f64 = 0.5;
/// Applicability answers at or above this Noul value count as applicable (spec FR-11).
pub const APPLICABILITY_THRESHOLD: f64 = 0.5;
/// The injection gate routes every decision to Needs architect at or above this value (spec FR-10, FR-14).
pub const INJECTION_THRESHOLD: f64 = 0.3;

/// Routing thresholds and per-criterion weights (spec FR-1, FR-14). A
/// criterion id missing from `weights` falls back to the catalogue's own
/// weight for that criterion — it is never treated as zero.
#[derive(Debug, Clone, PartialEq)]
pub struct RoutingConfig {
    pub threshold: f64,
    pub min_margin: f64,
    pub weights: BTreeMap<String, f64>,
}

/// Errors composing or evaluating a decision type's answers.
#[derive(Debug, Error, PartialEq)]
pub enum ScoringError {
    #[error("missing answer for '{0}'")]
    MissingAnswer(String),
    #[error("answer for '{0}' was not the expected question type")]
    WrongAnswerType(String),
    #[error("decision type '{type_id}': choice named an option that doesn't exist: '{option_id}'")]
    UnknownOption { type_id: String, option_id: String },
}

/// Resolves criterion `id`'s weight: `weights[id]` if present, otherwise the
/// catalogue's own weight for that criterion (spec "missing -> 0 weight is
/// WRONG — fall back to the catalogue weight").
fn resolve_weight(cat: &Catalog, weights: &BTreeMap<String, f64>, id: &str) -> f64 {
    if let Some(w) = weights.get(id) {
        return *w;
    }
    cat.criteria
        .iter()
        .find(|c| c.id == id)
        .map(|c| c.weight)
        .unwrap_or(0.0)
}

/// Composes each scored option's criterion scores and weighted composite
/// (spec FR-14): `composite(option) = Σ w_c · (score_c / 4) / Σ w_c`. Only
/// options that were actually asked Score questions (the eligible non-hold
/// options, per `questions::decision_questions`) are included. Sorted by
/// composite descending, then option id ascending.
pub fn compose(
    cat: &Catalog,
    dt: &DecisionType,
    answers: &BTreeMap<String, Answer>,
    weights: &BTreeMap<String, f64>,
) -> Result<Vec<OptionScore>, ScoringError> {
    let mut scores = Vec::new();

    for opt in &dt.options {
        let mut criterion_scores = BTreeMap::new();
        for criterion in &cat.criteria {
            let key = format!("score__{}__{}__{}", dt.id, opt.id, criterion.id);
            if let Some(answer) = answers.get(&key) {
                let score = answer
                    .as_score()
                    .ok_or_else(|| ScoringError::WrongAnswerType(key.clone()))?;
                criterion_scores.insert(criterion.id.clone(), score);
            }
        }

        if criterion_scores.is_empty() {
            // No Score questions were asked for this option (a hold option,
            // or one excluded by eligibility) — it isn't scored at all.
            continue;
        }
        if criterion_scores.len() != cat.criteria.len() {
            let missing_criterion = cat
                .criteria
                .iter()
                .find(|c| !criterion_scores.contains_key(&c.id))
                .expect("criterion_scores is a strict subset of cat.criteria here");
            return Err(ScoringError::MissingAnswer(format!(
                "score__{}__{}__{}",
                dt.id, opt.id, missing_criterion.id
            )));
        }

        let mut weighted_sum = 0.0;
        let mut weight_total = 0.0;
        for criterion in &cat.criteria {
            let w = resolve_weight(cat, weights, &criterion.id);
            let score = criterion_scores[&criterion.id];
            weighted_sum += w * (score / 4.0);
            weight_total += w;
        }
        let composite = if weight_total > 0.0 {
            weighted_sum / weight_total
        } else {
            0.0
        };

        scores.push(OptionScore {
            option_id: opt.id.clone(),
            criterion_scores,
            composite,
        });
    }

    scores.sort_by(|a, b| {
        b.composite
            .partial_cmp(&a.composite)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.option_id.cmp(&b.option_id))
    });

    Ok(scores)
}

/// Routes one decision (spec FR-14): Proposed iff none of the reason codes
/// fire. `scores` must already be sorted by composite descending (as
/// [`compose`] returns them).
///
/// - `low_confidence` if `confidence < cfg.threshold`.
/// - `disagreement` if the top-composite option isn't `choice` (when several
///   options tie for the top composite, this only fires if `choice` isn't
///   among them).
/// - `close_margin` if the gap between the top two composites is less than
///   `cfg.min_margin`, or the top composites tie. With zero or one scored
///   option there is no second option to compare against, so `close_margin`
///   never fires in that case.
/// - `hold_option` if `choice_ring` is `Ring::Hold`.
/// - `possible_injection` if `injection` is true.
///
/// Reason codes are returned sorted and deduplicated.
pub fn route(
    choice: &str,
    confidence: f64,
    choice_ring: Ring,
    scores: &[OptionScore],
    injection: bool,
    cfg: &RoutingConfig,
) -> (Route, Vec<ReasonCode>) {
    let mut reasons = Vec::new();

    if confidence < cfg.threshold {
        reasons.push(ReasonCode::LowConfidence);
    }

    if let Some(top) = scores.first() {
        let tied_at_top = scores
            .iter()
            .take_while(|s| s.composite == top.composite)
            .count();
        let agree = scores
            .iter()
            .take(tied_at_top)
            .any(|s| s.option_id == choice);
        if !agree {
            reasons.push(ReasonCode::Disagreement);
        }

        if let Some(runner_up) = scores.get(1) {
            let margin = top.composite - runner_up.composite;
            if tied_at_top > 1 || margin < cfg.min_margin {
                reasons.push(ReasonCode::CloseMargin);
            }
        }
        // A single scored option has no alternative to compare against, so
        // close_margin never fires for it.
    }

    if choice_ring == Ring::Hold {
        reasons.push(ReasonCode::HoldOption);
    }

    if injection {
        reasons.push(ReasonCode::PossibleInjection);
    }

    reasons.sort();
    reasons.dedup();

    let route = if reasons.is_empty() {
        Route::Proposed
    } else {
        Route::NeedsArchitect
    };
    (route, reasons)
}

/// One decision type's full evaluation: its Choice answer, composed option
/// scores, and the resulting route + reasons.
#[derive(Debug, Clone, PartialEq)]
pub struct TypeEvaluation {
    pub choice: String,
    pub confidence: f64,
    pub probabilities: BTreeMap<String, f64>,
    pub option_scores: Vec<OptionScore>,
    pub route: Route,
    pub reasons: Vec<ReasonCode>,
}

/// Reads `choice__<type>` and its Score answers out of `answers`, composes
/// and routes them (spec FR-14). `eligible_ids` restricts composed scores to
/// the options that were actually eligible for this decision (spec FR-12);
/// an answer for any other option, if present, is ignored.
pub fn evaluate_type(
    cat: &Catalog,
    dt: &DecisionType,
    answers: &BTreeMap<String, Answer>,
    eligible_ids: &[String],
    injection: bool,
    cfg: &RoutingConfig,
) -> Result<TypeEvaluation, ScoringError> {
    let choice_key = format!("choice__{}", dt.id);
    let choice_answer = answers
        .get(&choice_key)
        .ok_or_else(|| ScoringError::MissingAnswer(choice_key.clone()))?;

    let (choice, confidence, probabilities) = match choice_answer {
        Answer::Choice {
            choice,
            confidence,
            probabilities,
        } => (choice.clone(), *confidence, probabilities.clone()),
        _ => return Err(ScoringError::WrongAnswerType(choice_key)),
    };

    let choice_ring = dt
        .options
        .iter()
        .find(|o| o.id == choice)
        .map(|o| o.ring)
        .ok_or_else(|| ScoringError::UnknownOption {
            type_id: dt.id.clone(),
            option_id: choice.clone(),
        })?;

    let option_scores: Vec<OptionScore> = compose(cat, dt, answers, &cfg.weights)?
        .into_iter()
        .filter(|s| eligible_ids.iter().any(|id| id == &s.option_id))
        .collect();

    let (route, reasons) = route(
        &choice,
        confidence,
        choice_ring,
        &option_scores,
        injection,
        cfg,
    );

    Ok(TypeEvaluation {
        choice,
        confidence,
        probabilities,
        option_scores,
        route,
        reasons,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::catalog::{Criterion, OptionDef};

    fn criterion(id: &str, weight: f64) -> Criterion {
        Criterion {
            id: id.to_string(),
            name: id.to_string(),
            weight,
            instructions: "x".to_string(),
            levels: vec![
                "l0".into(),
                "l1".into(),
                "l2".into(),
                "l3".into(),
                "l4".into(),
            ],
        }
    }

    fn option(id: &str, ring: Ring) -> OptionDef {
        OptionDef {
            id: id.to_string(),
            name: id.to_string(),
            ring,
            description: "x".to_string(),
            aliases: vec![],
        }
    }

    fn catalog_with(criteria: Vec<Criterion>, options: Vec<OptionDef>) -> (Catalog, DecisionType) {
        let dt = DecisionType {
            id: "widget".to_string(),
            name: "Widget".to_string(),
            group: None,
            source: None,
            matrix: None,
            question: "Which widget?".to_string(),
            options,
            baseline: false,
        };
        let cat = Catalog {
            types: vec![dt.clone()],
            criteria,
            rules: crate::model::catalog::Rules {
                groups: vec![],
                auth_app_types: BTreeMap::new(),
            },
            adr_template: String::new(),
        };
        (cat, dt)
    }

    fn score_answer(score: f64) -> Answer {
        Answer::Score {
            score,
            confidence: 0.9,
            probabilities: BTreeMap::new(),
            legend: None,
        }
    }

    fn choice_answer(
        choice: &str,
        confidence: f64,
        probabilities: BTreeMap<String, f64>,
    ) -> Answer {
        Answer::Choice {
            choice: choice.to_string(),
            confidence,
            probabilities,
        }
    }

    fn default_config(weights: BTreeMap<String, f64>) -> RoutingConfig {
        RoutingConfig {
            threshold: 0.5,
            min_margin: 0.05,
            weights,
        }
    }

    #[test]
    fn compose_matches_hand_computed_composite_with_equal_weights() {
        let (cat, dt) = catalog_with(
            vec![criterion("a", 1.0), criterion("b", 1.0)],
            vec![option("x", Ring::Adopt), option("y", Ring::Trial)],
        );
        let mut answers = BTreeMap::new();
        answers.insert("score__widget__x__a".to_string(), score_answer(4.0));
        answers.insert("score__widget__x__b".to_string(), score_answer(2.0));
        answers.insert("score__widget__y__a".to_string(), score_answer(1.0));
        answers.insert("score__widget__y__b".to_string(), score_answer(1.0));

        let scores = compose(&cat, &dt, &answers, &BTreeMap::new()).expect("should compose");

        // x: (1*(4/4) + 1*(2/4)) / 2 = (1.0 + 0.5) / 2 = 0.75
        // y: (1*(1/4) + 1*(1/4)) / 2 = (0.25 + 0.25) / 2 = 0.25
        assert_eq!(scores.len(), 2);
        assert_eq!(scores[0].option_id, "x");
        assert!(
            (scores[0].composite - 0.75).abs() < 1e-9,
            "got {}",
            scores[0].composite
        );
        assert_eq!(scores[1].option_id, "y");
        assert!(
            (scores[1].composite - 0.25).abs() < 1e-9,
            "got {}",
            scores[1].composite
        );
    }

    #[test]
    fn compose_weights_criteria_unevenly_matching_hand_computed_figures() {
        let (cat, dt) = catalog_with(
            vec![criterion("a", 3.0), criterion("b", 1.0)],
            vec![option("x", Ring::Adopt)],
        );
        let mut answers = BTreeMap::new();
        answers.insert("score__widget__x__a".to_string(), score_answer(4.0));
        answers.insert("score__widget__x__b".to_string(), score_answer(0.0));

        let scores = compose(&cat, &dt, &answers, &BTreeMap::new()).expect("should compose");
        // (3*(4/4) + 1*(0/4)) / 4 = (3.0 + 0.0) / 4 = 0.75
        assert!(
            (scores[0].composite - 0.75).abs() < 1e-9,
            "got {}",
            scores[0].composite
        );
    }

    #[test]
    fn compose_falls_back_to_catalogue_weight_when_a_criterion_is_missing_from_overrides() {
        let (cat, dt) = catalog_with(
            vec![criterion("a", 1.0), criterion("b", 3.0)],
            vec![option("x", Ring::Adopt)],
        );
        let mut answers = BTreeMap::new();
        answers.insert("score__widget__x__a".to_string(), score_answer(4.0));
        answers.insert("score__widget__x__b".to_string(), score_answer(0.0));

        // Override only "a"; "b" must fall back to the catalogue's weight of 3.0,
        // not be treated as 0 (which would give a composite of 1.0, not 0.25).
        let mut overrides = BTreeMap::new();
        overrides.insert("a".to_string(), 1.0);

        let scores = compose(&cat, &dt, &answers, &overrides).expect("should compose");
        // (1*(4/4) + 3*(0/4)) / 4 = 1.0 / 4 = 0.25
        assert!(
            (scores[0].composite - 0.25).abs() < 1e-9,
            "got {}",
            scores[0].composite
        );
    }

    #[test]
    fn compose_skips_options_with_no_score_answers_at_all() {
        let (cat, dt) = catalog_with(
            vec![criterion("a", 1.0)],
            vec![option("x", Ring::Adopt), option("h", Ring::Hold)],
        );
        let mut answers = BTreeMap::new();
        answers.insert("score__widget__x__a".to_string(), score_answer(4.0));
        // no score for "h": it was never asked (hold, not mentioned).

        let scores = compose(&cat, &dt, &answers, &BTreeMap::new()).expect("should compose");
        assert_eq!(scores.len(), 1);
        assert_eq!(scores[0].option_id, "x");
    }

    #[test]
    fn compose_errors_when_an_option_has_a_partial_score_set() {
        let (cat, dt) = catalog_with(
            vec![criterion("a", 1.0), criterion("b", 1.0)],
            vec![option("x", Ring::Adopt)],
        );
        let mut answers = BTreeMap::new();
        answers.insert("score__widget__x__a".to_string(), score_answer(4.0));
        // "b" is missing entirely.

        let err = compose(&cat, &dt, &answers, &BTreeMap::new()).unwrap_err();
        assert_eq!(
            err,
            ScoringError::MissingAnswer("score__widget__x__b".to_string())
        );
    }

    fn scores(pairs: &[(&str, f64)]) -> Vec<OptionScore> {
        pairs
            .iter()
            .map(|(id, composite)| OptionScore {
                option_id: id.to_string(),
                criterion_scores: BTreeMap::new(),
                composite: *composite,
            })
            .collect()
    }

    #[test]
    fn route_is_proposed_when_nothing_is_wrong() {
        let cfg = default_config(BTreeMap::new());
        let (r, reasons) = route(
            "x",
            0.9,
            Ring::Adopt,
            &scores(&[("x", 0.8), ("y", 0.2)]),
            false,
            &cfg,
        );
        assert_eq!(r, Route::Proposed);
        assert!(reasons.is_empty());
    }

    #[test]
    fn route_flags_low_confidence_alone() {
        let cfg = default_config(BTreeMap::new());
        let (r, reasons) = route(
            "x",
            0.3,
            Ring::Adopt,
            &scores(&[("x", 0.8), ("y", 0.2)]),
            false,
            &cfg,
        );
        assert_eq!(r, Route::NeedsArchitect);
        assert_eq!(reasons, vec![ReasonCode::LowConfidence]);
    }

    #[test]
    fn route_flags_disagreement_alone_when_choice_is_not_the_top_composite() {
        let cfg = default_config(BTreeMap::new());
        let (r, reasons) = route(
            "y",
            0.9,
            Ring::Trial,
            &scores(&[("x", 0.8), ("y", 0.2)]),
            false,
            &cfg,
        );
        assert_eq!(r, Route::NeedsArchitect);
        assert_eq!(reasons, vec![ReasonCode::Disagreement]);
    }

    #[test]
    fn route_flags_close_margin_alone_when_the_top_two_are_within_min_margin() {
        let cfg = default_config(BTreeMap::new());
        let (r, reasons) = route(
            "x",
            0.9,
            Ring::Adopt,
            &scores(&[("x", 0.51), ("y", 0.50)]),
            false,
            &cfg,
        );
        assert_eq!(r, Route::NeedsArchitect);
        assert_eq!(reasons, vec![ReasonCode::CloseMargin]);
    }

    #[test]
    fn route_flags_hold_option_alone() {
        let cfg = default_config(BTreeMap::new());
        let (r, reasons) = route(
            "x",
            0.9,
            Ring::Hold,
            &scores(&[("x", 0.8), ("y", 0.2)]),
            false,
            &cfg,
        );
        assert_eq!(r, Route::NeedsArchitect);
        assert_eq!(reasons, vec![ReasonCode::HoldOption]);
    }

    #[test]
    fn route_flags_possible_injection_alone() {
        let cfg = default_config(BTreeMap::new());
        let (r, reasons) = route(
            "x",
            0.9,
            Ring::Adopt,
            &scores(&[("x", 0.8), ("y", 0.2)]),
            true,
            &cfg,
        );
        assert_eq!(r, Route::NeedsArchitect);
        assert_eq!(reasons, vec![ReasonCode::PossibleInjection]);
    }

    #[test]
    fn route_combines_every_reason_at_once_sorted_and_deduped() {
        let cfg = default_config(BTreeMap::new());
        // low confidence, disagreement (choice "z" isn't top), close margin
        // (0.51 vs 0.50), hold ring, and injection — all at once.
        let (r, reasons) = route(
            "z",
            0.1,
            Ring::Hold,
            &scores(&[("x", 0.51), ("y", 0.50)]),
            true,
            &cfg,
        );
        assert_eq!(r, Route::NeedsArchitect);
        assert_eq!(
            reasons,
            vec![
                ReasonCode::LowConfidence,
                ReasonCode::Disagreement,
                ReasonCode::CloseMargin,
                ReasonCode::HoldOption,
                ReasonCode::PossibleInjection,
            ]
        );
    }

    #[test]
    fn route_treats_a_tie_at_the_top_as_close_margin_and_disagreement_unless_choice_is_tied() {
        let cfg = default_config(BTreeMap::new());
        let tied = scores(&[("x", 0.5), ("y", 0.5)]);

        // choice is one of the tied leaders: no disagreement, but still close_margin.
        let (_, reasons) = route("x", 0.9, Ring::Adopt, &tied, false, &cfg);
        assert_eq!(reasons, vec![ReasonCode::CloseMargin]);

        // choice is not among the tied leaders: both disagreement and close_margin.
        let (_, reasons) = route("z", 0.9, Ring::Adopt, &tied, false, &cfg);
        assert_eq!(
            reasons,
            vec![ReasonCode::Disagreement, ReasonCode::CloseMargin]
        );
    }

    #[test]
    fn route_never_flags_close_margin_with_only_one_scored_option() {
        let cfg = default_config(BTreeMap::new());
        let (_, reasons) = route("x", 0.9, Ring::Adopt, &scores(&[("x", 0.9)]), false, &cfg);
        assert!(!reasons.contains(&ReasonCode::CloseMargin));
    }

    #[test]
    fn a_hold_option_is_never_proposed_even_when_everything_else_agrees() {
        let cfg = default_config(BTreeMap::new());
        let (r, _) = route(
            "x",
            0.99,
            Ring::Hold,
            &scores(&[("x", 0.9), ("y", 0.1)]),
            false,
            &cfg,
        );
        assert_eq!(r, Route::NeedsArchitect);
    }

    #[test]
    fn evaluate_type_reads_choice_and_composes_and_routes() {
        let (cat, dt) = catalog_with(
            vec![criterion("a", 1.0)],
            vec![option("x", Ring::Adopt), option("y", Ring::Trial)],
        );
        let mut probs = BTreeMap::new();
        probs.insert("x".to_string(), 0.9);
        probs.insert("y".to_string(), 0.1);

        let mut answers = BTreeMap::new();
        answers.insert(
            "choice__widget".to_string(),
            choice_answer("x", 0.9, probs.clone()),
        );
        answers.insert("score__widget__x__a".to_string(), score_answer(4.0));
        answers.insert("score__widget__y__a".to_string(), score_answer(0.0));

        let cfg = default_config(BTreeMap::new());
        let eligible_ids = vec!["x".to_string(), "y".to_string()];
        let eval = evaluate_type(&cat, &dt, &answers, &eligible_ids, false, &cfg)
            .expect("should evaluate");

        assert_eq!(eval.choice, "x");
        assert_eq!(eval.confidence, 0.9);
        assert_eq!(eval.probabilities, probs);
        assert_eq!(eval.route, Route::Proposed);
        assert!(eval.reasons.is_empty());
        assert_eq!(eval.option_scores.len(), 2);
    }

    #[test]
    fn evaluate_type_errors_when_the_choice_answer_is_missing() {
        let (cat, dt) = catalog_with(vec![criterion("a", 1.0)], vec![option("x", Ring::Adopt)]);
        let answers = BTreeMap::new();
        let cfg = default_config(BTreeMap::new());
        let err = evaluate_type(&cat, &dt, &answers, &["x".to_string()], false, &cfg).unwrap_err();
        assert_eq!(
            err,
            ScoringError::MissingAnswer("choice__widget".to_string())
        );
    }

    #[test]
    fn evaluate_type_errors_when_the_choice_names_an_unknown_option() {
        let (cat, dt) = catalog_with(vec![criterion("a", 1.0)], vec![option("x", Ring::Adopt)]);
        let mut answers = BTreeMap::new();
        answers.insert(
            "choice__widget".to_string(),
            choice_answer("does-not-exist", 0.9, BTreeMap::new()),
        );
        answers.insert("score__widget__x__a".to_string(), score_answer(4.0));
        let cfg = default_config(BTreeMap::new());
        let err = evaluate_type(&cat, &dt, &answers, &["x".to_string()], false, &cfg).unwrap_err();
        assert_eq!(
            err,
            ScoringError::UnknownOption {
                type_id: "widget".to_string(),
                option_id: "does-not-exist".to_string(),
            }
        );
    }
}
