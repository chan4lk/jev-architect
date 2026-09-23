//! Brief extraction.
//!
//! - Mode A: free text goes straight to MiniCPM, through
//!   `LocalModel::chat_json` constrained to the Brief JSON schema, and comes
//!   back as a `Brief`.
//! - Mode B small path (FR-7): the document sections are the evidence; the
//!   Brief's `context` comes from Jev's context Choices, mapped here by
//!   [`context_from_answers`] (the Jev call itself is made by `pipeline`).
//! - Mode B large path (FR-7): MiniCPM extracts a partial Brief per section
//!   ([`extract_brief_mode_b_large`]); the partials are tagged with their
//!   section id and merged in code ([`merge_partial_briefs`]).

use std::collections::BTreeMap;

use serde::de::DeserializeOwned;
use serde_json::Value;
use thiserror::Error;

use crate::catalog::option_mentioned;
use crate::docs::Section;
use crate::jev::Answer;
use crate::model::brief::{
    brief_json_schema, Brief, BriefItem, Budget, Compliance, ContextAssessment, DataSensitivity,
    Scale, TeamSize, Timeline,
};
use crate::model::catalog::Catalog;
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
            Ok(mut brief) => {
                // Mode A has no sections to cite; the model sometimes fills
                // `sources` with quotes, which would read as citations.
                for item in items_mut(&mut brief) {
                    item.sources.clear();
                }
                return Ok(brief);
            }
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

fn items_mut(brief: &mut Brief) -> impl Iterator<Item = &mut BriefItem> {
    brief
        .requirements
        .iter_mut()
        .chain(brief.nfrs.iter_mut())
        .chain(brief.constraints.iter_mut())
        .chain(brief.team_skills.iter_mut())
}

// ---------------------------------------------------------------------
// Mode B
// ---------------------------------------------------------------------

/// The system prompt for Mode B large-path extraction: one section of a
/// longer document at a time, producing a *partial* Brief.
pub const MODE_B_SYSTEM_PROMPT: &str = r#"You are extracting a partial structured project brief from ONE section of a longer requirements document.

Extract ONLY what is explicitly stated in this section. Never guess, infer, or assume a value that is not stated. Other sections are handled separately, so leave out anything this section does not state. For any Context Assessment dimension that this section does not explicitly state, use "unknown" (or, for compliance, an empty list).

Context Assessment dimensions and their values:
- scale: "small" (fewer than 1,000 users), "medium" (fewer than 100,000 users), "large" (100,000+ users), or "unknown".
- budget: "tight" (less than $500/mo), "moderate" (less than $5,000/mo), "enterprise" (more than $5,000/mo), or "unknown".
- timeline: "urgent" (less than 4 weeks), "normal" (1-3 months), "long_term" (3+ months), or "unknown".
- team_size: "solo_pair" (a solo developer or a pair), "small" (3-5 people), "large" (5+ people), or "unknown".
- compliance: a list of zero or more of "soc2", "gdpr", "hipaa", "industry_specific". An empty list means none was stated.
- data_sensitivity: "public", "internal", "confidential", "restricted", or "unknown".

Also extract:
- summary: one short sentence describing the project, or an empty string if this section does not describe the project as a whole.
- requirements, nfrs, constraints, team_skills: atomic, short, plain-language items stated in this section — one fact per item. Leave every item's "sources" as an empty list.
- mentioned_technologies: every specific technology, product, language, framework, or platform named in this section, listed once each.

Output JSON only, matching the given schema exactly. No prose, no markdown, no commentary outside the JSON."#;

/// The result of Mode B large-path extraction: the merged Brief, plus a note
/// for every section that failed extraction and was skipped.
#[derive(Debug, Clone, PartialEq)]
pub struct ModeBLarge {
    pub brief: Brief,
    pub notes: Vec<String>,
}

/// Mode B large path (FR-7): asks the local model for a partial Brief per
/// section, in document order, then merges them with
/// [`merge_partial_briefs`]. A section whose extraction fails (transport
/// error, or JSON that doesn't parse as a Brief) is retried once; if it
/// fails again it is skipped and recorded in `notes`. If every section
/// fails, the whole extraction fails with `BriefExtractionFailed`.
///
/// `on_section(done, total)` is called after each section is processed.
pub async fn extract_brief_mode_b_large(
    model: &dyn LocalModel,
    sections: &[Section],
    fallback_summary: &str,
    on_section: &(dyn Fn(usize, usize) + Sync),
) -> Result<ModeBLarge, BriefError> {
    let schema = brief_json_schema();
    let mut partials = Vec::new();
    let mut notes = Vec::new();

    for (i, section) in sections.iter().enumerate() {
        let user = match &section.heading {
            Some(h) => format!("Section {} — {}\n\n{}", section.id, h, section.text),
            None => format!("Section {}\n\n{}", section.id, section.text),
        };

        let mut last_reason = String::new();
        let mut partial = None;
        for _attempt in 0..2 {
            match model.chat_json(MODE_B_SYSTEM_PROMPT, &user, &schema).await {
                Ok(raw) => match serde_json::from_str::<Brief>(&raw) {
                    Ok(brief) => {
                        partial = Some(brief);
                        break;
                    }
                    Err(e) => last_reason = e.to_string(),
                },
                Err(e) => last_reason = e.to_string(),
            }
        }
        match partial {
            Some(brief) => partials.push((section.id.clone(), brief)),
            None => notes.push(format!(
                "{}: extraction failed and the section was skipped ({last_reason})",
                section.id
            )),
        }
        on_section(i + 1, sections.len());
    }

    if partials.is_empty() {
        return Err(BriefError::BriefExtractionFailed {
            reason: format!("every section failed extraction: {}", notes.join("; ")),
        });
    }

    let brief = merge_partial_briefs(partials, fallback_summary);
    validate_brief(&brief).map_err(|reason| BriefError::BriefExtractionFailed { reason })?;
    Ok(ModeBLarge { brief, notes })
}

/// Merges per-section partial Briefs (in document order) into one Brief
/// (FR-7). Every item's `sources` is overwritten with its section id —
/// whatever the model put there is discarded. Items are de-duplicated by
/// case-insensitive trimmed text, with their sources unioned. Each context
/// dimension takes the first non-`unknown` value in document order;
/// compliance and `mentioned_technologies` are unioned. The summary is the
/// first non-empty partial summary, or `fallback_summary`.
pub fn merge_partial_briefs(partials: Vec<(String, Brief)>, fallback_summary: &str) -> Brief {
    let mut merged = Brief {
        summary: String::new(),
        context: ContextAssessment::default(),
        requirements: Vec::new(),
        nfrs: Vec::new(),
        constraints: Vec::new(),
        team_skills: Vec::new(),
        mentioned_technologies: Vec::new(),
    };

    for (section_id, partial) in partials {
        if merged.summary.is_empty() {
            merged.summary = partial.summary.trim().to_string();
        }

        let ctx = &mut merged.context;
        let p = partial.context;
        if ctx.scale == Scale::Unknown {
            ctx.scale = p.scale;
        }
        if ctx.budget == Budget::Unknown {
            ctx.budget = p.budget;
        }
        if ctx.timeline == Timeline::Unknown {
            ctx.timeline = p.timeline;
        }
        if ctx.team_size == TeamSize::Unknown {
            ctx.team_size = p.team_size;
        }
        if ctx.data_sensitivity == DataSensitivity::Unknown {
            ctx.data_sensitivity = p.data_sensitivity;
        }
        for c in p.compliance {
            if !ctx.compliance.contains(&c) {
                ctx.compliance.push(c);
            }
        }

        merge_items(&mut merged.requirements, partial.requirements, &section_id);
        merge_items(&mut merged.nfrs, partial.nfrs, &section_id);
        merge_items(&mut merged.constraints, partial.constraints, &section_id);
        merge_items(&mut merged.team_skills, partial.team_skills, &section_id);
        union_technologies(&mut merged.mentioned_technologies, partial.mentioned_technologies);
    }

    if merged.summary.is_empty() {
        merged.summary = fallback_summary.trim().to_string();
    }
    merged
}

fn dedupe_key(text: &str) -> String {
    text.trim().to_lowercase()
}

fn merge_items(target: &mut Vec<BriefItem>, items: Vec<BriefItem>, section_id: &str) {
    for item in items {
        let key = dedupe_key(&item.text);
        if key.is_empty() {
            continue;
        }
        match target.iter_mut().find(|t| dedupe_key(&t.text) == key) {
            Some(existing) => {
                if !existing.sources.iter().any(|s| s == section_id) {
                    existing.sources.push(section_id.to_string());
                }
            }
            None => target.push(BriefItem {
                text: item.text.trim().to_string(),
                sources: vec![section_id.to_string()],
            }),
        }
    }
}

/// Appends each of `more` to `target` unless it is already there
/// (case-insensitive, trimmed).
pub fn union_technologies(target: &mut Vec<String>, more: Vec<String>) {
    for tech in more {
        let key = dedupe_key(&tech);
        if !key.is_empty() && !target.iter().any(|t| dedupe_key(t) == key) {
            target.push(tech.trim().to_string());
        }
    }
}

/// The catalogue options named anywhere in `text` (FR-7): every option whose
/// name or an alias appears as a whole word/phrase, per
/// `catalog::option_mentioned`. Returns option names in catalogue order, each
/// once. Deterministic — no model involved.
pub fn scan_mentioned_technologies(cat: &Catalog, text: &str) -> Vec<String> {
    let hay = [text.to_string()];
    let mut out = Vec::new();
    for dt in &cat.types {
        for opt in &dt.options {
            if option_mentioned(opt, &hay) {
                union_technologies(&mut out, vec![opt.name.clone()]);
            }
        }
    }
    out
}

/// Maps the answers to `questions::context_questions()` onto a
/// `ContextAssessment` (Mode B small path, FR-7): a `not_stated` Choice is
/// `unknown`; a compliance regime is included when its Noul is ≥ 0.5.
pub fn context_from_answers(answers: &BTreeMap<String, Answer>) -> Result<ContextAssessment, String> {
    let mut compliance = Vec::new();
    for c in [
        Compliance::Soc2,
        Compliance::Gdpr,
        Compliance::Hipaa,
        Compliance::IndustrySpecific,
    ] {
        let name = serde_json::to_value(&c)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default();
        let key = format!("context__compliance__{name}");
        let p = answers
            .get(&key)
            .and_then(Answer::as_noul)
            .ok_or_else(|| format!("missing Noul answer for '{key}'"))?;
        if p >= 0.5 {
            compliance.push(c);
        }
    }

    Ok(ContextAssessment {
        scale: context_choice(answers, "context__scale")?,
        budget: context_choice(answers, "context__budget")?,
        timeline: context_choice(answers, "context__timeline")?,
        team_size: context_choice(answers, "context__team_size")?,
        compliance,
        data_sensitivity: context_choice(answers, "context__data_sensitivity")?,
    })
}

fn context_choice<T: DeserializeOwned + Default>(
    answers: &BTreeMap<String, Answer>,
    key: &str,
) -> Result<T, String> {
    let choice = answers
        .get(key)
        .and_then(Answer::as_choice)
        .ok_or_else(|| format!("missing Choice answer for '{key}'"))?;
    if choice == "not_stated" {
        return Ok(T::default());
    }
    serde_json::from_value(Value::String(choice.to_string()))
        .map_err(|_| format!("'{key}' was answered with an option that was not offered: '{choice}'"))
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

    #[tokio::test]
    async fn mode_a_clears_sources_the_model_filled_in() {
        let json = serde_json::json!({
            "summary": "A tool.",
            "context": {
                "scale": "unknown", "budget": "unknown", "timeline": "unknown",
                "team_size": "unknown", "compliance": [], "data_sensitivity": "unknown"
            },
            "requirements": [{"text": "Export", "sources": ["\"we need export\""]}],
            "nfrs": [{"text": "Fast", "sources": ["quote"]}],
            "constraints": [], "team_skills": [], "mentioned_technologies": []
        })
        .to_string();
        let model = FakeLocalModel::new("m", true, vec![Ok(json)]);
        let brief = extract_brief_mode_a(&model, "text").await.unwrap();
        assert!(brief.requirements[0].sources.is_empty());
        assert!(brief.nfrs[0].sources.is_empty());
    }

    fn partial(summary: &str, scale: &str, reqs: &[&str], techs: &[&str]) -> Brief {
        serde_json::from_value(serde_json::json!({
            "summary": summary,
            "context": {
                "scale": scale, "budget": "unknown", "timeline": "unknown",
                "team_size": "unknown", "compliance": ["gdpr"], "data_sensitivity": "unknown"
            },
            "requirements": reqs.iter().map(|r| serde_json::json!({"text": r, "sources": ["bogus"]})).collect::<Vec<_>>(),
            "nfrs": [], "constraints": [], "team_skills": [],
            "mentioned_technologies": techs,
        }))
        .unwrap()
    }

    #[test]
    fn merge_tags_sources_dedupes_and_takes_first_known_context() {
        let merged = merge_partial_briefs(
            vec![
                ("S1".to_string(), partial("", "unknown", &["SSO login", "Audit log"], &["Azure"])),
                ("S2".to_string(), partial("Portal.", "medium", &["  sso LOGIN "], &["azure", "SignalR"])),
                ("S3".to_string(), partial("Other.", "large", &["Audit log"], &[])),
            ],
            "Doc title",
        );
        assert_eq!(merged.summary, "Portal.");
        assert_eq!(merged.context.scale, Scale::Medium);
        assert_eq!(merged.context.compliance, vec![Compliance::Gdpr]);
        assert_eq!(merged.requirements.len(), 2);
        assert_eq!(merged.requirements[0].text, "SSO login");
        assert_eq!(merged.requirements[0].sources, vec!["S1", "S2"]);
        assert_eq!(merged.requirements[1].sources, vec!["S1", "S3"]);
        assert_eq!(merged.mentioned_technologies, vec!["Azure", "SignalR"]);
    }

    #[test]
    fn merge_falls_back_to_the_doc_title_for_summary() {
        let merged = merge_partial_briefs(vec![("S1".to_string(), partial(" ", "unknown", &[], &[]))], "Doc title");
        assert_eq!(merged.summary, "Doc title");
    }

    #[tokio::test]
    async fn mode_b_large_skips_a_section_that_fails_twice() {
        let ok = serde_json::to_string(&partial("P.", "small", &["Req"], &[])).unwrap();
        let sections: Vec<Section> = (1..=3)
            .map(|i| Section { id: format!("S{i}"), heading: None, text: "t".to_string(), tokens: 1 })
            .collect();
        let model = FakeLocalModel::new(
            "m",
            true,
            vec![Ok(ok.clone()), Ok("bad".to_string()), Err(LocalModelError::Timeout), Ok(ok)],
        );
        let progress = std::sync::Mutex::new(Vec::new());
        let out = extract_brief_mode_b_large(&model, &sections, "Title", &|done, total| {
            progress.lock().unwrap().push((done, total))
        })
        .await
        .unwrap();
        assert_eq!(model.calls().len(), 4);
        assert_eq!(out.notes.len(), 1);
        assert!(out.notes[0].starts_with("S2:"));
        assert_eq!(out.brief.requirements[0].sources, vec!["S1", "S3"]);
        assert_eq!(*progress.lock().unwrap(), vec![(1, 3), (2, 3), (3, 3)]);
    }

    #[tokio::test]
    async fn mode_b_large_fails_when_every_section_fails() {
        let sections = vec![Section { id: "S1".to_string(), heading: None, text: "t".to_string(), tokens: 1 }];
        let model = FakeLocalModel::new("m", true, vec![Ok("x".to_string()), Ok("y".to_string())]);
        let err = extract_brief_mode_b_large(&model, &sections, "Title", &|_, _| {}).await.unwrap_err();
        assert!(matches!(err, BriefError::BriefExtractionFailed { .. }));
    }

    #[test]
    fn context_from_answers_maps_not_stated_to_unknown_and_compliance_by_threshold() {
        let choice = |c: &str| Answer::Choice {
            choice: c.to_string(),
            confidence: 0.9,
            probabilities: BTreeMap::new(),
        };
        let mut answers = BTreeMap::new();
        answers.insert("context__scale".to_string(), choice("medium"));
        answers.insert("context__budget".to_string(), choice("tight"));
        answers.insert("context__timeline".to_string(), choice("not_stated"));
        answers.insert("context__team_size".to_string(), choice("not_stated"));
        answers.insert("context__data_sensitivity".to_string(), choice("confidential"));
        for (k, v) in [("soc2", 0.1), ("gdpr", 0.5), ("hipaa", 0.49), ("industry_specific", 0.0)] {
            answers.insert(format!("context__compliance__{k}"), Answer::Noul { noul: v });
        }
        let ctx = context_from_answers(&answers).unwrap();
        assert_eq!(ctx.scale, Scale::Medium);
        assert_eq!(ctx.budget, Budget::Tight);
        assert_eq!(ctx.timeline, Timeline::Unknown);
        assert_eq!(ctx.team_size, TeamSize::Unknown);
        assert_eq!(ctx.data_sensitivity, DataSensitivity::Confidential);
        assert_eq!(ctx.compliance, vec![Compliance::Gdpr]);

        answers.insert("context__scale".to_string(), choice("huge"));
        assert!(context_from_answers(&answers).is_err());
    }

    #[test]
    fn scan_finds_catalogue_options_by_alias_once_each() {
        let cat = Catalog::bundled().unwrap();
        let found = scan_mentioned_technologies(&cat, "We run PostgreSQL on Azure, plus Postgres replicas and SignalR.");
        assert!(found.contains(&"PostgreSQL".to_string()));
        assert!(found.contains(&"Microsoft Azure".to_string()));
        assert!(found.contains(&"SignalR (.NET)".to_string()));
        let unique: std::collections::BTreeSet<_> = found.iter().collect();
        assert_eq!(unique.len(), found.len());
    }
}
