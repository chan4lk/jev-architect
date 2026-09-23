//! Serde DTOs and validation for the review/approval flow (spec FR-18).
//!
//! `Review` rows are append-only (spec AC-13): the store enforces this with
//! a DB trigger, and `adr_status` always looks at the *latest* review rather
//! than assuming there is only one.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// An approval action taken on a decision (spec FR-18).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewAction {
  Accept,
  Override,
  Reject,
}

/// A recorded review row, as stored (append-only; spec AC-13).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Review {
  pub id: i64,
  pub decision_id: String,
  pub action: ReviewAction,
  pub option_id: Option<String>,
  pub reviewer: String,
  pub reason: Option<String>,
  pub at_utc: DateTime<Utc>,
}

/// The input to record a new review (spec FR-18); `Store::append_review`
/// validates it, stamps `at_utc`, and assigns the row's `id`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewReview {
  pub decision_id: String,
  pub action: ReviewAction,
  pub option_id: Option<String>,
  pub reviewer: String,
  pub reason: Option<String>,
}

/// Why a `NewReview` failed validation (spec AC-13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ReviewError {
  #[error("reviewer name must not be blank")]
  BlankReviewer,
  #[error("a reason is required for {0:?}")]
  ReasonRequired(ReviewAction),
  #[error("an option is required for an override")]
  OptionRequired,
}

/// Validates a `NewReview` against spec AC-13's rules: the reviewer name
/// must be non-blank for every action; `Override` and `Reject` require a
/// non-blank reason; `Override` additionally requires an option id.
pub fn validate_review(r: &NewReview) -> Result<(), ReviewError> {
  if r.reviewer.trim().is_empty() {
    return Err(ReviewError::BlankReviewer);
  }

  if r.action == ReviewAction::Override && is_blank(&r.option_id) {
    return Err(ReviewError::OptionRequired);
  }

  if matches!(r.action, ReviewAction::Override | ReviewAction::Reject) && is_blank(&r.reason) {
    return Err(ReviewError::ReasonRequired(r.action));
  }

  Ok(())
}

fn is_blank(s: &Option<String>) -> bool {
  s.as_deref().map(str::trim).unwrap_or("").is_empty()
}

/// The ADR status implied by a decision's review history (spec FR-18).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdrStatus {
  ProposedAi,
  Accepted,
  AcceptedOverride,
  Rejected,
}

impl AdrStatus {
  pub fn label(&self) -> &'static str {
    match self {
      AdrStatus::ProposedAi => "Proposed (AI) — pending approval",
      AdrStatus::Accepted => "Accepted",
      AdrStatus::AcceptedOverride => "Accepted (override)",
      AdrStatus::Rejected => "Rejected",
    }
  }
}

/// The ADR status of the *latest* review, ordered by `(at_utc, id)`
/// (spec FR-18). `ProposedAi` when `reviews` is empty.
pub fn adr_status(reviews: &[Review]) -> AdrStatus {
  match reviews.iter().max_by_key(|r| (r.at_utc, r.id)) {
    None => AdrStatus::ProposedAi,
    Some(r) => match r.action {
      ReviewAction::Accept => AdrStatus::Accepted,
      ReviewAction::Override => AdrStatus::AcceptedOverride,
      ReviewAction::Reject => AdrStatus::Rejected,
    },
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use chrono::TimeZone;

  fn new_review(action: ReviewAction, option_id: Option<&str>, reviewer: &str, reason: Option<&str>) -> NewReview {
    NewReview {
      decision_id: "d1".to_string(),
      action,
      option_id: option_id.map(str::to_string),
      reviewer: reviewer.to_string(),
      reason: reason.map(str::to_string),
    }
  }

  #[test]
  fn accept_requires_only_a_non_blank_reviewer() {
    assert!(validate_review(&new_review(ReviewAction::Accept, None, "Jane", None)).is_ok());
    assert_eq!(
      validate_review(&new_review(ReviewAction::Accept, None, "  ", None)),
      Err(ReviewError::BlankReviewer)
    );
    assert_eq!(
      validate_review(&new_review(ReviewAction::Accept, None, "", None)),
      Err(ReviewError::BlankReviewer)
    );
  }

  #[test]
  fn reject_requires_a_reason_but_no_option() {
    assert!(validate_review(&new_review(ReviewAction::Reject, None, "Jane", Some("too costly"))).is_ok());
    assert_eq!(
      validate_review(&new_review(ReviewAction::Reject, None, "Jane", None)),
      Err(ReviewError::ReasonRequired(ReviewAction::Reject))
    );
    assert_eq!(
      validate_review(&new_review(ReviewAction::Reject, None, "Jane", Some("   "))),
      Err(ReviewError::ReasonRequired(ReviewAction::Reject))
    );
  }

  #[test]
  fn override_requires_both_option_and_reason() {
    assert!(validate_review(&new_review(ReviewAction::Override, Some("hetzner"), "Jane", Some("cheaper"))).is_ok());
    assert_eq!(
      validate_review(&new_review(ReviewAction::Override, None, "Jane", Some("cheaper"))),
      Err(ReviewError::OptionRequired)
    );
    assert_eq!(
      validate_review(&new_review(ReviewAction::Override, Some("hetzner"), "Jane", None)),
      Err(ReviewError::ReasonRequired(ReviewAction::Override))
    );
  }

  fn review(id: i64, action: ReviewAction, secs: i64) -> Review {
    Review {
      id,
      decision_id: "d1".to_string(),
      action,
      option_id: if action == ReviewAction::Override {
        Some("hetzner".to_string())
      } else {
        None
      },
      reviewer: "Jane".to_string(),
      reason: None,
      at_utc: Utc.timestamp_opt(secs, 0).unwrap(),
    }
  }

  #[test]
  fn adr_status_is_proposed_ai_with_no_reviews() {
    assert_eq!(adr_status(&[]), AdrStatus::ProposedAi);
    assert_eq!(AdrStatus::ProposedAi.label(), "Proposed (AI) — pending approval");
  }

  #[test]
  fn adr_status_follows_the_latest_review_by_at_utc() {
    let reviews = vec![review(1, ReviewAction::Accept, 100), review(2, ReviewAction::Reject, 200)];
    assert_eq!(adr_status(&reviews), AdrStatus::Rejected);
    assert_eq!(AdrStatus::Rejected.label(), "Rejected");
  }

  #[test]
  fn adr_status_breaks_at_utc_ties_by_id() {
    // Same timestamp, different ids: the higher id (inserted later) wins.
    let reviews = vec![review(1, ReviewAction::Accept, 100), review(2, ReviewAction::Override, 100)];
    assert_eq!(adr_status(&reviews), AdrStatus::AcceptedOverride);
  }

  #[test]
  fn adr_status_accepted_label() {
    assert_eq!(AdrStatus::Accepted.label(), "Accepted");
    assert_eq!(AdrStatus::AcceptedOverride.label(), "Accepted (override)");
  }
}
