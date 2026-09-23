//! Brief extraction.
//!
//! Mode A (this file, so far): free text goes straight to MiniCPM, through
//! `LocalModel::chat_json` constrained to the Brief JSON schema, and comes
//! back as a `Brief`. Mode B (document upload, small/large paths) is added
//! by a later task in this same file.

use thiserror::Error;

use crate::model::brief::{brief_json_schema, Brief};
use crate::ollama::{LocalModel, LocalModelError};

/// The system prompt for Mode A extraction.
///
/// States the Context Assessment value definitions verbatim so the model
/// has the same wording a question builder would use, and is explicit that
/// `unknown` (or an empty list, for compliance) is the correct answer for
/// anything not stated — never a guess.
pub const MODE_A_SYSTEM_PROMPT: &str = r#"You are extracting a structured project brief from free text written by a software architect.

Extract ONLY what is explicitly stated in the text. Never guess, infer, or assume a value that is not stated. For any Context Assessment dimension that is not explicitly stated, use "unknown" (or, for compliance, an empty list). Do not fill in a plausible-sounding default.

Context Assessment dimensions and their values:
- scale: "small" (fewer than 1,000 users), "medium" (fewer than 100,000 users), "large" (100,000+ users), or "unknown".
- budget: "tight" (less than $500/mo), "moderate" (less than $5,000/mo), "enterprise" (more than $5,000/mo), or "unknown".
- timeline: "urgent" (less than 4 weeks), "normal" (1-3 months), "long_term" (3+ months), or "unknown".
- team_size: "solo_pair" (a solo developer or a pair), "small" (3-5 people), "large" (5+ people), or "unknown".
- compliance: a list of zero or more of "soc2", "gdpr", "hipaa", "industry_specific". An empty list means none was stated.
- data_sensitivity: "public", "internal", "confidential", "restricted", or "unknown".

Also extract:
- summary: a short (1-3 sentence) plain-language summary of the project.
- requirements: atomic, short, plain-language items — one requirement per item.
- nfrs: atomic, short, plain-language non-functional requirements.
- constraints: atomic, short, plain-language constraints.
- team_skills: atomic, short, plain-language notes about the team's existing skills.
- mentioned_technologies: every specific technology, product, language, framework, or platform named anywhere in the text, listed once each.

Keep every item atomic (one fact per item) and short. Output JSON only, matching the given schema exactly. No prose, no markdown, no commentary outside the JSON."#;

/// Errors from extracting a Brief.
#[derive(Debug, Error)]
pub enum BriefError {
    #[error(transparent)]
    LocalModel(#[from] LocalModelError),
    #[error("brief extraction failed: {reason}")]
    BriefExtractionFailed { reason: String },
}

/// Validates a `Brief` beyond what serde/schema already enforce.
pub fn validate_brief(brief: &Brief) -> Result<(), String> {
    if brief.summary.trim().is_empty() {
        return Err("summary must not be empty".to_string());
    }
    Ok(())
}

/// Extracts a `Brief` from free text via Mode A (FR-5).
///
/// Calls `model.chat_json` with the Brief JSON schema, then parses and
/// validates the result. A transport error (`LocalModelError`) propagates
/// immediately and is not retried here — retry-on-transport-failure, if
/// any, is the caller's concern. A parse or validation failure is retried
/// exactly once; a second failure is reported as
/// `BriefError::BriefExtractionFailed` (AC-5).
pub async fn extract_brief_mode_a(model: &dyn LocalModel, text: &str) -> Result<Brief, BriefError> {
    let schema = brief_json_schema();

    let mut last_reason = String::new();
    for _attempt in 0..2 {
        let raw = model.chat_json(MODE_A_SYSTEM_PROMPT, text, &schema).await?;
        match parse_and_validate(&raw) {
            Ok(brief) => return Ok(brief),
            Err(reason) => last_reason = reason,
        }
    }

    Err(BriefError::BriefExtractionFailed {
        reason: last_reason,
    })
}

fn parse_and_validate(raw: &str) -> Result<Brief, String> {
    let brief: Brief = serde_json::from_str(raw).map_err(|e| e.to_string())?;
    validate_brief(&brief)?;
    Ok(brief)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ollama::FakeLocalModel;

    fn valid_brief_json() -> String {
        serde_json::json!({
            "summary": "A small internal reporting tool.",
            "context": {
                "scale": "small",
                "budget": "tight",
                "timeline": "urgent",
                "team_size": "solo_pair",
                "compliance": [],
                "data_sensitivity": "internal"
            },
            "requirements": [{"text": "Export to CSV", "sources": []}],
            "nfrs": [],
            "constraints": [],
            "team_skills": [],
            "mentioned_technologies": ["PostgreSQL"]
        })
        .to_string()
    }

    #[tokio::test]
    async fn valid_json_produces_a_brief_in_one_call() {
        let model = FakeLocalModel::new("test-model", true, vec![Ok(valid_brief_json())]);
        let brief = extract_brief_mode_a(&model, "some free text").await.unwrap();
        assert_eq!(brief.summary, "A small internal reporting tool.");
        assert_eq!(model.calls().len(), 1);
    }

    #[tokio::test]
    async fn malformed_then_valid_succeeds_with_exactly_two_calls() {
        let model = FakeLocalModel::new(
            "test-model",
            true,
            vec![Ok("not json".to_string()), Ok(valid_brief_json())],
        );
        let brief = extract_brief_mode_a(&model, "some free text").await.unwrap();
        assert_eq!(brief.summary, "A small internal reporting tool.");
        assert_eq!(model.calls().len(), 2);
    }

    #[tokio::test]
    async fn malformed_twice_fails_with_exactly_two_calls() {
        let model = FakeLocalModel::new(
            "test-model",
            true,
            vec![Ok("not json".to_string()), Ok("still not json".to_string())],
        );
        let err = extract_brief_mode_a(&model, "some free text").await.unwrap_err();
        assert!(matches!(err, BriefError::BriefExtractionFailed { .. }));
        assert_eq!(model.calls().len(), 2);
    }

    #[tokio::test]
    async fn empty_summary_fails_validation_and_retries() {
        let mostly_valid = serde_json::json!({
            "summary": "",
            "context": {
                "scale": "unknown", "budget": "unknown", "timeline": "unknown",
                "team_size": "unknown", "compliance": [], "data_sensitivity": "unknown"
            },
            "requirements": [], "nfrs": [], "constraints": [], "team_skills": [],
            "mentioned_technologies": []
        })
        .to_string();
        let model = FakeLocalModel::new(
            "test-model",
            true,
            vec![Ok(mostly_valid.clone()), Ok(mostly_valid)],
        );
        let err = extract_brief_mode_a(&model, "text").await.unwrap_err();
        assert!(matches!(err, BriefError::BriefExtractionFailed { .. }));
        assert_eq!(model.calls().len(), 2);
    }

    #[tokio::test]
    async fn transport_error_propagates_without_retry() {
        let model = FakeLocalModel::new(
            "test-model",
            true,
            vec![Err(LocalModelError::Timeout), Ok(valid_brief_json())],
        );
        let err = extract_brief_mode_a(&model, "text").await.unwrap_err();
        assert!(matches!(err, BriefError::LocalModel(LocalModelError::Timeout)));
        assert_eq!(model.calls().len(), 1);
    }

    #[test]
    fn validate_brief_rejects_empty_summary() {
        let mut brief: Brief = serde_json::from_str(&valid_brief_json()).unwrap();
        brief.summary = "  ".to_string();
        assert!(validate_brief(&brief).is_err());
    }
}
