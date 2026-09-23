//! Stage orchestration (design "Technical Approach"): brief → (user confirms)
//! → gates → platforms → rules + applicability → decisions → done.
//!
//! Every stage persists its result to `stage_results`; a stage whose result
//! already exists is skipped, which is both the Retry mechanism (re-run
//! `run_decisions` after a failure) and why History replay
//! ([`build_report`]) never calls Jev (AC-16). Every Jev call is logged to
//! `jev_calls`. Arithmetic, rules and routing stay in `scoring` / `rules`;
//! this module only wires them together.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::path::Path;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use futures::stream::{self, StreamExt};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

use crate::brief::{
    context_from_answers, extract_brief_mode_a, extract_brief_mode_b_large, scan_mentioned_technologies,
    union_technologies, validate_brief, BriefError,
};
use crate::docs::{estimate_tokens, parse_document, total_tokens, DocError, Section};
use crate::jev::{Answer, DecisionClient, DecisionRequest, JevError, Question};
use crate::model::brief::{Brief, ContextAssessment};
use crate::model::catalog::{Catalog, DecisionType};
use crate::model::decision::{DecisionResult, NotApplicable, Route};
use crate::model::report::{CriterionView, DecisionView, Gates, NotApplicableView, OptionView, Report};
use crate::model::review::adr_status;
use crate::model::settings::Settings;
use crate::ollama::{LocalModel, LocalModelError};
use crate::questions::{
    applicability_questions, batch, context_questions, decision_questions, eligible_options, gate_questions,
    platform_questions, validate_questions,
};
use crate::rules::{candidate_types, PlatformOutcome};
use crate::scoring::{
    evaluate_type, RoutingConfig, ScoringError, APPLICABILITY_THRESHOLD, GATE_THRESHOLD, INJECTION_THRESHOLD,
};
use crate::store::{SessionRow, Store, StoreError};

/// Minimum length of a Mode A description (spec edge case "Empty or tiny input").
pub const MIN_DESCRIBE_CHARS: usize = 20;
/// How many accepted decisions of a type are offered as precedent (FR-13).
pub const PRECEDENT_LIMIT: usize = 3;

const STAGE_BRIEF: &str = "brief";
const STAGE_GATES: &str = "gates";
const STAGE_PLATFORMS: &str = "platforms";
const STAGE_APPLICABILITY: &str = "applicability";
const STAGE_DECISIONS: &str = "decisions";
const STAGE_REVIEW: &str = "review";
const STAGE_DONE: &str = "done";
const STAGE_OUT_OF_REMIT: &str = "out_of_remit";
/// Every stage whose result depends on the brief, in run order.
const POST_BRIEF_STAGES: [&str; 4] = [STAGE_GATES, STAGE_PLATFORMS, STAGE_APPLICABILITY, STAGE_DECISIONS];
const PLATFORM_TYPES: [&str; 2] = ["cloud-platform", "backend-platform"];

// ---------------------------------------------------------------------
// Dependencies, progress, errors
// ---------------------------------------------------------------------

/// Everything the pipeline needs. Settings are read from `store` at the
/// start of each operation.
#[derive(Clone)]
pub struct Deps {
    pub catalog: Arc<Catalog>,
    pub store: Arc<Store>,
    pub jev: Arc<dyn DecisionClient>,
    pub local: Arc<dyn LocalModel>,
}

/// A progress event (IPC contract `Progress`, event `pipeline://progress`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Progress {
    pub session_id: String,
    pub stage: String,
    pub done: usize,
    pub total: usize,
    pub message: Option<String>,
}

/// Receives progress events (FR-15). `commands` forwards them as Tauri events.
pub trait ProgressSink: Send + Sync {
    fn emit(&self, p: Progress);
}

/// Discards every event.
pub struct NoopSink;

impl ProgressSink for NoopSink {
    fn emit(&self, _p: Progress) {}
}

/// Records every event (tests).
#[derive(Default)]
pub struct RecordingSink {
    events: Mutex<Vec<Progress>>,
}

impl RecordingSink {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn events(&self) -> Vec<Progress> {
        self.events.lock().expect("sink mutex poisoned").clone()
    }
}

impl ProgressSink for RecordingSink {
    fn emit(&self, p: Progress) {
        self.events.lock().expect("sink mutex poisoned").push(p);
    }
}

/// Pipeline errors. [`PipelineError::code`] gives the IPC contract's error
/// code, and [`PipelineError::stage`] the stage that failed, if any.
#[derive(Debug, Error)]
pub enum PipelineError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("{0}")]
    Validation(String),
    #[error("brief extraction failed: {0}")]
    BriefExtractionFailed(String),
    #[error("Jev call failed: {0}")]
    Jev(String),
    #[error("no OpenRouter API key is configured")]
    NoApiKey,
    #[error("local model error: {0}")]
    LocalModel(String),
    #[error(transparent)]
    Document(#[from] DocError),
    #[error("store error: {0}")]
    Store(String),
    #[error("the request is out of remit: it does not ask for a software technology decision")]
    OutOfRemit,
    #[error("the data notice must be acknowledged before any content is sent to Jev")]
    DataNoticeRequired,
    #[error("internal error: {0}")]
    Internal(String),
    /// Any of the above, raised while running `stage`.
    #[error("{source}")]
    InStage {
        stage: String,
        source: Box<PipelineError>,
    },
}

impl PipelineError {
    /// The IPC contract error code.
    pub fn code(&self) -> &'static str {
        match self {
            PipelineError::NotFound(_) => "not_found",
            PipelineError::Validation(_) => "validation",
            PipelineError::BriefExtractionFailed(_) => "brief_extraction_failed",
            PipelineError::Jev(_) => "jev",
            PipelineError::NoApiKey => "no_api_key",
            PipelineError::LocalModel(_) => "local_model",
            PipelineError::Document(_) => "document",
            PipelineError::Store(_) => "store",
            PipelineError::OutOfRemit => "out_of_remit",
            PipelineError::DataNoticeRequired => "data_notice_required",
            PipelineError::Internal(_) => "internal",
            PipelineError::InStage { source, .. } => source.code(),
        }
    }

    /// The stage that failed, when the error came from a pipeline stage.
    pub fn stage(&self) -> Option<&str> {
        match self {
            PipelineError::InStage { stage, .. } => Some(stage),
            _ => None,
        }
    }

    fn in_stage(self, stage: &str) -> Self {
        match self {
            e @ PipelineError::InStage { .. } => e,
            e => PipelineError::InStage {
                stage: stage.to_string(),
                source: Box::new(e),
            },
        }
    }
}

impl From<StoreError> for PipelineError {
    fn from(e: StoreError) -> Self {
        PipelineError::Store(e.to_string())
    }
}

impl From<JevError> for PipelineError {
    fn from(e: JevError) -> Self {
        match e {
            JevError::NoApiKey => PipelineError::NoApiKey,
            e => PipelineError::Jev(e.to_string()),
        }
    }
}

impl From<LocalModelError> for PipelineError {
    fn from(e: LocalModelError) -> Self {
        PipelineError::LocalModel(e.to_string())
    }
}

impl From<BriefError> for PipelineError {
    fn from(e: BriefError) -> Self {
        match e {
            BriefError::LocalModel(e) => e.into(),
            BriefError::BriefExtractionFailed { reason } => PipelineError::BriefExtractionFailed(reason),
        }
    }
}

impl From<ScoringError> for PipelineError {
    /// A scoring error means Jev's answers didn't fit the questions asked
    /// (edge case: "the stage fails with a typed error. Nothing is guessed").
    fn from(e: ScoringError) -> Self {
        PipelineError::Jev(e.to_string())
    }
}

impl From<serde_json::Error> for PipelineError {
    fn from(e: serde_json::Error) -> Self {
        PipelineError::Internal(format!("json: {e}"))
    }
}

// ---------------------------------------------------------------------
// Session DTOs (IPC contract Session / SessionSummary / SessionView)
// ---------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Session {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub title: String,
    pub mode: String,
    pub input_text: Option<String>,
    pub doc_name: Option<String>,
    pub stage: String,
    pub error: Option<String>,
    pub brief_confirmed_at: Option<DateTime<Utc>>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SessionSummary {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub title: String,
    pub mode: String,
    pub stage: String,
    pub decision_count: usize,
    pub needs_architect_count: usize,
    pub unreviewed_count: usize,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SessionView {
    pub session: Session,
    pub sections: Option<Vec<Section>>,
    pub doc_path: Option<String>,
    pub brief: Option<Brief>,
    pub report: Option<Report>,
}

// ---------------------------------------------------------------------
// Persisted stage results
// ---------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
struct BriefStage {
    /// `"small"` / `"large"` for uploads, `None` for Mode A.
    doc_path: Option<String>,
    /// Per-section extraction failures (large path).
    #[serde(default)]
    notes: Vec<String>,
    /// Set when a confirmed brief is edited: it must be confirmed again.
    #[serde(default)]
    needs_reconfirm: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct GatesStage {
    gates: Gates,
    injection: bool,
    out_of_remit: bool,
    truncated: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct PlatformsStage {
    cloud: String,
    backend: String,
    auth_app_type: String,
    decision_ids: Vec<String>,
    truncated: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct ApplicabilityStage {
    candidates: Vec<String>,
    applicable: Vec<String>,
    not_applicable: Vec<NotApplicable>,
    truncated: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct SkippedType {
    type_id: String,
    reason: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct DecisionsStage {
    decision_ids: Vec<String>,
    skipped: Vec<SkippedType>,
    truncated: bool,
}

fn get_stage<T: DeserializeOwned>(d: &Deps, id: &str, stage: &str) -> Result<Option<T>, PipelineError> {
    match d.store.get_stage_result(id, stage) {
        Some(v) => Ok(Some(serde_json::from_value(v)?)),
        None => Ok(None),
    }
}

fn put_stage<T: Serialize>(d: &Deps, id: &str, stage: &str, v: &T) -> Result<(), PipelineError> {
    d.store.put_stage_result(id, stage, &serde_json::to_value(v)?)?;
    Ok(())
}

// ---------------------------------------------------------------------
// Budget fitting
// ---------------------------------------------------------------------

/// A decision-call `state`, fitted to the token budget.
#[derive(Debug, Clone, PartialEq)]
pub struct FittedState {
    pub state: Value,
    /// True when evidence sections or brief requirements had to be removed
    /// (dropping precedent alone is not a truncation of the user's input).
    pub truncated: bool,
    /// The ids of the evidence sections that were kept, in document order.
    pub section_ids: Vec<String>,
}

/// Builds `{ brief, evidence_sections?, bistec_standard?, bistec_precedent? }`
/// and fits it to `budget` tokens (design "Jev request shapes"), measured as
/// `estimate_tokens(serde_json::to_string(&state))`. While over budget it
/// removes, in order: all precedent; then evidence sections from the end;
/// then brief requirements from the end. Constraints and NFRs are never
/// removed; if the state still doesn't fit, it is a `validation` error.
pub fn fit_state(
    brief: &Brief,
    sections: &[Section],
    standard: Option<&Value>,
    precedent: Vec<Value>,
    budget: usize,
) -> Result<FittedState, PipelineError> {
    let mut brief = brief.clone();
    let mut sections: Vec<&Section> = sections.iter().collect();
    let mut precedent = precedent;
    let mut truncated = false;

    loop {
        let mut state = json!({ "brief": brief });
        if !sections.is_empty() {
            state["evidence_sections"] = sections_json(sections.iter().copied());
        }
        if let Some(standard) = standard {
            state["bistec_standard"] = standard.clone();
        }
        if !precedent.is_empty() {
            state["bistec_precedent"] = Value::Array(precedent.clone());
        }

        if estimate_tokens(&serde_json::to_string(&state)?) <= budget {
            return Ok(FittedState {
                state,
                truncated,
                section_ids: sections.iter().map(|s| s.id.clone()).collect(),
            });
        }

        if !precedent.is_empty() {
            precedent.clear();
        } else if !sections.is_empty() {
            sections.pop();
            truncated = true;
        } else if !brief.requirements.is_empty() {
            brief.requirements.pop();
            truncated = true;
        } else {
            return Err(PipelineError::Validation(format!(
                "state exceeds the {budget}-token budget even without precedent, evidence, or requirements"
            )));
        }
    }
}

fn sections_json<'a>(sections: impl Iterator<Item = &'a Section>) -> Value {
    Value::Array(
        sections
            .map(|s| json!({ "id": s.id, "heading": s.heading, "text": s.text }))
            .collect(),
    )
}

// ---------------------------------------------------------------------
// Jev calls
// ---------------------------------------------------------------------

struct Asked {
    answers: BTreeMap<String, Answer>,
    requests: Vec<DecisionRequest>,
    model: String,
}

/// Sends `questions` against `state` in batches of `max_questions_per_call`,
/// one batch after another, logging each call's usage. Fails if any answer
/// key is missing or unexpected.
async fn ask(
    d: &Deps,
    session_id: &str,
    stage: &str,
    settings: &Settings,
    state: &Value,
    questions: BTreeMap<String, Question>,
) -> Result<Asked, PipelineError> {
    validate_questions(&questions).map_err(PipelineError::Internal)?;

    let mut asked = Asked {
        answers: BTreeMap::new(),
        requests: Vec::new(),
        model: String::new(),
    };
    for questions in batch(questions, settings.max_questions_per_call.max(1)) {
        let req = DecisionRequest {
            model: settings.jev_model.clone(),
            state: state.clone(),
            questions,
        };
        let resp = d.jev.decide(&req).await?;
        d.store.log_jev_call(
            session_id,
            stage,
            resp.usage.input_tokens,
            resp.usage.output_tokens,
            resp.usage.cost,
            &resp.model,
        )?;

        let missing: Vec<String> = req.questions.keys().filter(|k| !resp.answers.contains_key(*k)).cloned().collect();
        let unexpected: Vec<String> = resp.answers.keys().filter(|k| !req.questions.contains_key(*k)).cloned().collect();
        if !missing.is_empty() || !unexpected.is_empty() {
            return Err(JevError::AnswerKeysMismatch { missing, unexpected }.into());
        }

        asked.answers.extend(resp.answers);
        asked.model = resp.model;
        asked.requests.push(req);
    }
    Ok(asked)
}

fn noul(answers: &BTreeMap<String, Answer>, key: &str) -> Result<f64, PipelineError> {
    answers
        .get(key)
        .and_then(Answer::as_noul)
        .ok_or_else(|| PipelineError::Jev(format!("missing Noul answer for '{key}'")))
}

/// sha256 hex of the serialized request(s) that produced a decision.
fn request_hash(requests: &[DecisionRequest]) -> Result<String, PipelineError> {
    let bytes = serde_json::to_vec(requests)?;
    Ok(hex::encode(Sha256::digest(&bytes)))
}

fn routing_config(settings: &Settings) -> RoutingConfig {
    RoutingConfig {
        threshold: settings.confidence_threshold,
        min_margin: settings.min_margin,
        weights: settings.weights.clone(),
    }
}

// ---------------------------------------------------------------------
// Session helpers
// ---------------------------------------------------------------------

fn session_row(d: &Deps, id: &str) -> Result<SessionRow, PipelineError> {
    d.store
        .get_session(id)
        .ok_or_else(|| PipelineError::NotFound(format!("session {id}")))
}

fn row_brief(row: &SessionRow) -> Result<Option<Brief>, PipelineError> {
    row.brief_json.as_deref().map(serde_json::from_str).transpose().map_err(Into::into)
}

fn row_sections(row: &SessionRow) -> Result<Option<Vec<Section>>, PipelineError> {
    row.sections_json.as_deref().map(serde_json::from_str).transpose().map_err(Into::into)
}

fn title_from(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= 60 {
        return flat;
    }
    let head: String = flat.chars().take(60).collect();
    format!("{}…", head.trim_end())
}

fn new_session(mode: &str, title: String, input_text: Option<String>, doc_name: Option<String>) -> SessionRow {
    SessionRow {
        id: Uuid::new_v4().to_string(),
        created_at: Utc::now(),
        title,
        mode: mode.to_string(),
        input_text,
        doc_name,
        sections_json: None,
        brief_json: None,
        brief_confirmed_at: None,
        stage: STAGE_BRIEF.to_string(),
        error: None,
    }
}

/// Records a brief-stage failure on the session and returns the error.
fn fail_brief(d: &Deps, id: &str, e: PipelineError) -> PipelineError {
    let failed = format!("failed:{STAGE_BRIEF}");
    let _ = d.store.set_session_stage(id, &failed, Some(&e.to_string()));
    e.in_stage(STAGE_BRIEF)
}

fn emit(sink: &dyn ProgressSink, id: &str, stage: &str, done: usize, total: usize, message: Option<String>) {
    sink.emit(Progress {
        session_id: id.to_string(),
        stage: stage.to_string(),
        done,
        total,
        message,
    });
}

// ---------------------------------------------------------------------
// Brief: Mode A and Mode B
// ---------------------------------------------------------------------

/// Mode A (FR-5): creates a session from free text and extracts its Brief
/// with the local model. The session ends at stage `review`, or
/// `failed:brief` (and the error is returned).
pub async fn start_describe(d: &Deps, text: &str) -> Result<String, PipelineError> {
    if text.trim().chars().count() < MIN_DESCRIBE_CHARS {
        return Err(PipelineError::Validation(format!(
            "describe the project in at least {MIN_DESCRIBE_CHARS} characters"
        )));
    }
    let row = new_session("describe", title_from(text), Some(text.to_string()), None);
    let id = row.id.clone();
    d.store.create_session(&row)?;

    let result: Result<(), PipelineError> = async {
        let brief = extract_brief_mode_a(d.local.as_ref(), text).await?;
        d.store.update_session_brief(&id, &serde_json::to_string(&brief)?)?;
        put_stage(d, &id, STAGE_BRIEF, &BriefStage::default())?;
        d.store.set_session_stage(&id, STAGE_REVIEW, None)?;
        Ok(())
    }
    .await;

    result.map_err(|e| fail_brief(d, &id, e))?;
    Ok(id)
}

/// Mode B (FR-6, FR-7): parses the document and routes by size.
///
/// - **small** (total tokens ≤ `state_token_budget`): the sections are the
///   evidence; the Brief's context comes from one Jev call of
///   `context_questions()` (so the data notice must already be acknowledged).
/// - **large**: the local model extracts a partial Brief per section, merged
///   in code; progress is emitted per section.
///
/// Both paths also scan the document for catalogue option names/aliases to
/// fill `mentioned_technologies`. The session ends at stage `review`.
pub async fn start_upload(d: &Deps, path: &Path, sink: &dyn ProgressSink) -> Result<String, PipelineError> {
    let settings = d.store.get_settings();
    let sections = parse_document(path)?;
    let small = total_tokens(&sections) <= settings.state_token_budget;
    if small && !d.store.data_notice_acked() {
        return Err(PipelineError::DataNoticeRequired);
    }

    let doc_title = sections
        .iter()
        .find_map(|s| s.heading.clone().filter(|h| !h.trim().is_empty()))
        .or_else(|| path.file_stem().and_then(|s| s.to_str()).map(str::to_string))
        .unwrap_or_else(|| "Uploaded document".to_string());
    let doc_name = path.file_name().and_then(|n| n.to_str()).map(str::to_string);

    let mut row = new_session("upload", title_from(&doc_title), None, doc_name);
    row.sections_json = Some(serde_json::to_string(&sections)?);
    let id = row.id.clone();
    d.store.create_session(&row)?;

    let doc_text = sections
        .iter()
        .map(|s| format!("{}\n{}", s.heading.as_deref().unwrap_or(""), s.text))
        .collect::<Vec<_>>()
        .join("\n\n");
    let scanned = scan_mentioned_technologies(&d.catalog, &doc_text);

    let result: Result<(), PipelineError> = async {
        let (brief, stage) = if small {
            emit(sink, &id, STAGE_BRIEF, 0, 1, Some("Reading project context".to_string()));
            let state = json!({ "document_sections": sections_json(sections.iter()) });
            let asked = ask(d, &id, STAGE_BRIEF, &settings, &state, context_questions()).await?;
            let context: ContextAssessment = context_from_answers(&asked.answers).map_err(PipelineError::Jev)?;
            emit(sink, &id, STAGE_BRIEF, 1, 1, None);
            let brief = Brief {
                summary: doc_title.clone(),
                context,
                requirements: Vec::new(),
                nfrs: Vec::new(),
                constraints: Vec::new(),
                team_skills: Vec::new(),
                mentioned_technologies: scanned,
            };
            let stage = BriefStage {
                doc_path: Some("small".to_string()),
                ..BriefStage::default()
            };
            (brief, stage)
        } else {
            let on_section = |done: usize, total: usize| {
                emit(sink, &id, STAGE_BRIEF, done, total, None);
            };
            let out = extract_brief_mode_b_large(d.local.as_ref(), &sections, &doc_title, &on_section).await?;
            let mut brief = out.brief;
            union_technologies(&mut brief.mentioned_technologies, scanned);
            let stage = BriefStage {
                doc_path: Some("large".to_string()),
                notes: out.notes,
                needs_reconfirm: false,
            };
            (brief, stage)
        };
        validate_brief(&brief).map_err(PipelineError::BriefExtractionFailed)?;
        d.store.update_session_brief(&id, &serde_json::to_string(&brief)?)?;
        put_stage(d, &id, STAGE_BRIEF, &stage)?;
        d.store.set_session_stage(&id, STAGE_REVIEW, None)?;
        Ok(())
    }
    .await;

    result.map_err(|e| fail_brief(d, &id, e))?;
    Ok(id)
}

/// Replaces the session's Brief (FR-8). Any post-brief stage results are
/// cleared so the decision pass re-runs from the gates on the edited
/// Brief, and the session goes back to stage `review`. Editing a confirmed
/// Brief requires confirming it again.
pub fn update_brief(d: &Deps, id: &str, brief: &Brief) -> Result<(), PipelineError> {
    let row = session_row(d, id)?;
    validate_brief(brief).map_err(PipelineError::Validation)?;

    d.store.update_session_brief(id, &serde_json::to_string(brief)?)?;
    d.store.clear_stage_results_from(id, &POST_BRIEF_STAGES)?;
    let mut stage: BriefStage = get_stage(d, id, STAGE_BRIEF)?.unwrap_or_default();
    stage.needs_reconfirm = row.brief_confirmed_at.is_some();
    put_stage(d, id, STAGE_BRIEF, &stage)?;
    d.store.set_session_stage(id, STAGE_REVIEW, None)?;
    Ok(())
}

/// Confirms the session's Brief (FR-8): nothing reaches the decision pass
/// without this.
pub fn confirm_brief(d: &Deps, id: &str) -> Result<(), PipelineError> {
    let row = session_row(d, id)?;
    if row.brief_json.is_none() {
        return Err(PipelineError::Validation("the session has no brief to confirm".to_string()));
    }
    d.store.confirm_brief(id)?;
    let mut stage: BriefStage = get_stage(d, id, STAGE_BRIEF)?.unwrap_or_default();
    stage.needs_reconfirm = false;
    put_stage(d, id, STAGE_BRIEF, &stage)?;
    Ok(())
}

// ---------------------------------------------------------------------
// Decision pass
// ---------------------------------------------------------------------

/// The evidence a session's decision calls are built from.
struct Evidence {
    brief: Brief,
    /// Small path: every section. Large path: the sections cited by any
    /// brief item, in document order. Mode A: none.
    sections: Vec<Section>,
    doc_path: Option<String>,
}

impl Evidence {
    fn load(row: &SessionRow, brief: Brief, doc_path: Option<String>) -> Result<Self, PipelineError> {
        let all = row_sections(row)?.unwrap_or_default();
        let sections = match doc_path.as_deref() {
            Some("small") => all,
            Some("large") => {
                let cited: BTreeSet<&str> = [&brief.requirements, &brief.nfrs, &brief.constraints, &brief.team_skills]
                    .into_iter()
                    .flatten()
                    .flat_map(|item| item.sources.iter().map(String::as_str))
                    .collect();
                all.into_iter().filter(|s| cited.contains(s.id.as_str())).collect()
            }
            _ => Vec::new(),
        };
        Ok(Evidence {
            brief,
            sections,
            doc_path,
        })
    }

    /// The section ids a decision cites: every cited section (large path),
    /// the sections actually sent (small path), or none (Mode A).
    fn cited(&self, fitted: &FittedState) -> Vec<String> {
        match self.doc_path.as_deref() {
            Some("large") => self.sections.iter().map(|s| s.id.clone()).collect(),
            Some("small") => fitted.section_ids.clone(),
            _ => Vec::new(),
        }
    }
}

struct Run<'a> {
    d: &'a Deps,
    id: &'a str,
    settings: Settings,
    ev: Evidence,
    sink: &'a dyn ProgressSink,
}

impl Run<'_> {
    fn emit(&self, stage: &str, done: usize, total: usize, message: Option<String>) {
        emit(self.sink, self.id, stage, done, total, message);
    }

    fn evidence_state(&self) -> Result<FittedState, PipelineError> {
        fit_state(&self.ev.brief, &self.ev.sections, None, Vec::new(), self.settings.state_token_budget)
    }

    fn decision(
        &self,
        dt: &DecisionType,
        answers: &BTreeMap<String, Answer>,
        eligible_ids: &[String],
        injection: bool,
        asked: &Asked,
        cited_sections: Vec<String>,
    ) -> Result<DecisionResult, PipelineError> {
        let eval = evaluate_type(
            &self.d.catalog,
            dt,
            answers,
            eligible_ids,
            injection,
            &routing_config(&self.settings),
        )?;
        Ok(DecisionResult {
            id: Uuid::new_v4().to_string(),
            session_id: self.id.to_string(),
            type_id: dt.id.clone(),
            choice: eval.choice,
            confidence: eval.confidence,
            probabilities: eval.probabilities,
            option_scores: eval.option_scores,
            route: eval.route,
            reasons: eval.reasons,
            model_snapshot: asked.model.clone(),
            request_hash: request_hash(&asked.requests)?,
            cited_sections,
        })
    }
}

/// Runs `f` as `stage` unless its result is already persisted (resume):
/// sets the session stage, persists the result, and on failure records
/// `failed:<stage>` with the error.
async fn staged<T, F, Fut>(run: &Run<'_>, stage: &str, f: F) -> Result<T, PipelineError>
where
    T: Serialize + DeserializeOwned,
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T, PipelineError>>,
{
    if let Some(done) = get_stage::<T>(run.d, run.id, stage)? {
        return Ok(done);
    }
    run.d.store.set_session_stage(run.id, stage, None)?;
    run.emit(stage, 0, 1, None);

    let result = match f().await {
        Ok(v) => put_stage(run.d, run.id, stage, &v).map(|_| v),
        Err(e) => Err(e),
    };
    match result {
        Ok(v) => {
            run.emit(stage, 1, 1, None);
            Ok(v)
        }
        Err(e) => {
            let _ = run
                .d
                .store
                .set_session_stage(run.id, &format!("failed:{stage}"), Some(&e.to_string()));
            Err(e.in_stage(stage))
        }
    }
}

/// Runs (or resumes) the decision pass (FR-9..FR-15). Requires a confirmed
/// Brief (`validation`) and an acknowledged data notice
/// (`data_notice_required`). Each stage is skipped if its result is already
/// persisted, so calling this again after a failure resumes from the failed
/// stage. Ends at stage `done`, or `out_of_remit` if the gates say the input
/// isn't a technical request (that is `Ok`, not an error).
pub async fn run_decisions(d: &Deps, id: &str, sink: &dyn ProgressSink) -> Result<(), PipelineError> {
    let row = session_row(d, id)?;
    let brief = row_brief(&row)?.ok_or_else(|| PipelineError::Validation("the session has no brief".to_string()))?;
    let brief_stage: BriefStage = get_stage(d, id, STAGE_BRIEF)?.unwrap_or_default();
    if row.brief_confirmed_at.is_none() || brief_stage.needs_reconfirm {
        return Err(PipelineError::Validation(
            "the brief must be confirmed before decisions run".to_string(),
        ));
    }
    if !d.store.data_notice_acked() {
        return Err(PipelineError::DataNoticeRequired);
    }

    let run = Run {
        d,
        id,
        settings: d.store.get_settings(),
        ev: Evidence::load(&row, brief, brief_stage.doc_path)?,
        sink,
    };

    let gates: GatesStage = staged(&run, STAGE_GATES, || gates_stage(&run)).await?;
    if gates.out_of_remit {
        d.store.set_session_stage(id, STAGE_OUT_OF_REMIT, None)?;
        run.emit(STAGE_OUT_OF_REMIT, 1, 1, None);
        return Ok(());
    }
    let platforms: PlatformsStage =
        staged(&run, STAGE_PLATFORMS, || platforms_stage(&run, gates.injection)).await?;
    let applicability: ApplicabilityStage =
        staged(&run, STAGE_APPLICABILITY, || applicability_stage(&run, &platforms)).await?;
    let _: DecisionsStage = staged(&run, STAGE_DECISIONS, || {
        decisions_stage(&run, &platforms, &applicability, gates.injection)
    })
    .await?;

    d.store.set_session_stage(id, STAGE_DONE, None)?;
    run.emit(STAGE_DONE, 1, 1, None);
    Ok(())
}

/// FR-10: three Nouls on the evidence.
async fn gates_stage(run: &Run<'_>) -> Result<GatesStage, PipelineError> {
    let fitted = run.evidence_state()?;
    let asked = ask(run.d, run.id, STAGE_GATES, &run.settings, &fitted.state, gate_questions()).await?;
    let gates = Gates {
        is_technical_request: noul(&asked.answers, "gate__is_technical_request")?,
        has_enough_context: noul(&asked.answers, "gate__has_enough_context")?,
        injection: noul(&asked.answers, "gate__injection")?,
    };
    Ok(GatesStage {
        injection: gates.injection >= INJECTION_THRESHOLD,
        out_of_remit: gates.is_technical_request < GATE_THRESHOLD,
        gates,
        truncated: fitted.truncated,
    })
}

/// FR-9 step 1: cloud-platform, backend-platform and the auth app type, in
/// one call (batched if over the per-call limit).
async fn platforms_stage(run: &Run<'_>, injection: bool) -> Result<PlatformsStage, PipelineError> {
    let cat = &run.d.catalog;
    let mentions = &run.ev.brief.mentioned_technologies;
    let fitted = run.evidence_state()?;
    let asked = ask(
        run.d,
        run.id,
        STAGE_PLATFORMS,
        &run.settings,
        &fitted.state,
        platform_questions(cat, mentions),
    )
    .await?;

    let auth_app_type = asked
        .answers
        .get("platform__auth_app_type")
        .and_then(Answer::as_choice)
        .ok_or_else(|| PipelineError::Jev("missing Choice answer for 'platform__auth_app_type'".to_string()))?
        .to_string();

    let mut decisions = Vec::new();
    for type_id in PLATFORM_TYPES {
        let dt = cat
            .type_by_id(type_id)
            .ok_or_else(|| PipelineError::Internal(format!("catalogue has no '{type_id}' type")))?;
        let eligible: Vec<String> = eligible_options(dt, mentions).iter().map(|o| o.id.clone()).collect();
        decisions.push(run.decision(dt, &asked.answers, &eligible, injection, &asked, run.ev.cited(&fitted))?);
    }
    for decision in &decisions {
        run.d.store.insert_decision(decision)?;
    }

    Ok(PlatformsStage {
        cloud: decisions[0].choice.clone(),
        backend: decisions[1].choice.clone(),
        auth_app_type,
        decision_ids: decisions.into_iter().map(|d| d.id).collect(),
        truncated: fitted.truncated,
    })
}

/// FR-9 step 2 + FR-11: rules select the candidate rows, then one Noul per
/// candidate decides which are applicable.
async fn applicability_stage(run: &Run<'_>, platforms: &PlatformsStage) -> Result<ApplicabilityStage, PipelineError> {
    let cat = &run.d.catalog;
    let outcome = PlatformOutcome {
        cloud: platforms.cloud.clone(),
        backend: platforms.backend.clone(),
        auth_app_type: platforms.auth_app_type.clone(),
    };
    let candidates = candidate_types(cat, &outcome, &run.ev.brief);
    let fitted = run.evidence_state()?;

    let mut applicable = Vec::new();
    let mut not_applicable = Vec::new();
    if !candidates.is_empty() {
        let asked = ask(
            run.d,
            run.id,
            STAGE_APPLICABILITY,
            &run.settings,
            &fitted.state,
            applicability_questions(cat, &candidates),
        )
        .await?;
        for type_id in &candidates {
            let p = noul(&asked.answers, &format!("applies__{type_id}"))?;
            if p >= APPLICABILITY_THRESHOLD {
                applicable.push(type_id.clone());
            } else {
                not_applicable.push(NotApplicable {
                    type_id: type_id.clone(),
                    probability: p,
                });
            }
        }
    }

    Ok(ApplicabilityStage {
        candidates,
        applicable,
        not_applicable,
        truncated: fitted.truncated,
    })
}

/// The `bistec_standard` block for a type's decision state: the type, its
/// question, and its eligible options with ring and description. Auth rows
/// also carry the Auth Decision Tree guidance for the decided app type (FR-9).
fn bistec_standard(cat: &Catalog, dt: &DecisionType, eligible_ids: &[String], platforms: &PlatformsStage) -> Value {
    let options: Vec<Value> = dt
        .options
        .iter()
        .filter(|o| eligible_ids.contains(&o.id))
        .map(|o| json!({ "id": o.id, "name": o.name, "ring": o.ring, "description": o.description }))
        .collect();
    let mut standard = json!({ "type": dt.name, "question": dt.question, "options": options });
    if dt.group.as_deref() == Some("auth") {
        if let Some(guidance) = cat.rules.auth_app_types.get(&platforms.auth_app_type) {
            standard["auth_decision_tree"] = json!({ "app_type": platforms.auth_app_type, "guidance": guidance });
        }
    }
    standard
}

/// FR-13: up to three recent accepted decisions of this type.
fn precedent(d: &Deps, dt: &DecisionType) -> Vec<Value> {
    d.store
        .precedent(&dt.id, PRECEDENT_LIMIT)
        .into_iter()
        .map(|p| {
            let name = dt
                .options
                .iter()
                .find(|o| o.id == p.option_id)
                .map(|o| o.name.clone())
                .unwrap_or_else(|| p.option_id.clone());
            json!({
                "option_id": p.option_id,
                "option_name": name,
                "summary": p.summary.lines().next().unwrap_or("").trim(),
            })
        })
        .collect()
}

/// One decision type's request(s), prepared by [`decisions_stage`].
struct Job<'c> {
    index: usize,
    dt: &'c DecisionType,
    eligible_ids: Vec<String>,
    questions: BTreeMap<String, Question>,
    fitted: FittedState,
}

async fn decide_job(
    run: &Run<'_>,
    job: Job<'_>,
    injection: bool,
) -> Result<(usize, String, DecisionResult), PipelineError> {
    let asked = ask(run.d, run.id, STAGE_DECISIONS, &run.settings, &job.fitted.state, job.questions).await?;
    let decision = run.decision(
        job.dt,
        &asked.answers,
        &job.eligible_ids,
        injection,
        &asked,
        run.ev.cited(&job.fitted),
    )?;
    Ok((job.index, job.dt.name.clone(), decision))
}

/// FR-12..FR-14: one request per applicable type (batched within a type if
/// needed), at most `max_concurrent_calls` in flight. Results are sorted by
/// catalogue order before persisting, so they never depend on completion
/// order. Nothing is persisted unless every type succeeds.
async fn decisions_stage(
    run: &Run<'_>,
    platforms: &PlatformsStage,
    applicability: &ApplicabilityStage,
    injection: bool,
) -> Result<DecisionsStage, PipelineError> {
    let cat = &run.d.catalog;
    let mentions = &run.ev.brief.mentioned_technologies;

    let mut jobs = Vec::new();
    let mut skipped = Vec::new();
    for (index, dt) in cat.types.iter().enumerate() {
        if !applicability.applicable.contains(&dt.id) {
            continue;
        }
        let eligible = eligible_options(dt, mentions);
        if eligible.is_empty() {
            skipped.push(SkippedType {
                type_id: dt.id.clone(),
                reason: "no_eligible_options".to_string(),
            });
            continue;
        }
        let eligible_ids: Vec<String> = eligible.iter().map(|o| o.id.clone()).collect();
        let standard = bistec_standard(cat, dt, &eligible_ids, platforms);
        let fitted = fit_state(
            &run.ev.brief,
            &run.ev.sections,
            Some(&standard),
            precedent(run.d, dt),
            run.settings.state_token_budget,
        )?;
        jobs.push(Job {
            index,
            dt,
            questions: decision_questions(cat, dt, &eligible),
            eligible_ids,
            fitted,
        });
    }

    let total = jobs.len();
    let truncated = jobs.iter().any(|j| j.fitted.truncated);
    run.emit(STAGE_DECISIONS, 0, total, None);

    // Futures are built eagerly from a named async fn (not a closure) so the
    // whole pipeline future stays `Send` for Tauri.
    let futures: Vec<_> = jobs.into_iter().map(|job| decide_job(run, job, injection)).collect();
    let mut in_flight = stream::iter(futures).buffer_unordered(run.settings.max_concurrent_calls.max(1));

    let mut results = Vec::with_capacity(total);
    while let Some(result) = in_flight.next().await {
        let (index, name, decision) = result?;
        results.push((index, decision));
        run.emit(STAGE_DECISIONS, results.len(), total, Some(name));
    }
    drop(in_flight);

    results.sort_by_key(|(index, _)| *index);
    for (_, decision) in &results {
        run.d.store.insert_decision(decision)?;
    }

    Ok(DecisionsStage {
        decision_ids: results.into_iter().map(|(_, d)| d.id).collect(),
        skipped,
        truncated,
    })
}

// ---------------------------------------------------------------------
// Replay: report, session view, history
// ---------------------------------------------------------------------

fn type_index(cat: &Catalog, type_id: &str) -> usize {
    cat.types.iter().position(|t| t.id == type_id).unwrap_or(usize::MAX)
}

/// The session's current decisions (those named by its persisted platform
/// and decision stage results), in catalogue order.
fn current_decisions(d: &Deps, id: &str) -> Result<Vec<DecisionResult>, PipelineError> {
    let mut ids: BTreeSet<String> = BTreeSet::new();
    if let Some(p) = get_stage::<PlatformsStage>(d, id, STAGE_PLATFORMS)? {
        ids.extend(p.decision_ids);
    }
    if let Some(s) = get_stage::<DecisionsStage>(d, id, STAGE_DECISIONS)? {
        ids.extend(s.decision_ids);
    }
    let mut decisions: Vec<DecisionResult> = d
        .store
        .decisions_for_session(id)
        .into_iter()
        .filter(|dr| ids.contains(&dr.id))
        .collect();
    decisions.sort_by_key(|dr| type_index(&d.catalog, &dr.type_id));
    Ok(decisions)
}

fn stage_truncated(d: &Deps, id: &str) -> Result<bool, PipelineError> {
    let mut truncated = false;
    for stage in POST_BRIEF_STAGES {
        if let Some(v) = get_stage::<Value>(d, id, stage)? {
            truncated |= v.get("truncated").and_then(Value::as_bool).unwrap_or(false);
        }
    }
    Ok(truncated)
}

/// Assembles the session's report purely from the store — never calls Jev
/// or the local model (AC-16). `None` until the decision pass has finished
/// (stage `done` or `out_of_remit`).
pub fn build_report(d: &Deps, id: &str) -> Result<Option<Report>, PipelineError> {
    let row = session_row(d, id)?;
    if row.stage != STAGE_DONE && row.stage != STAGE_OUT_OF_REMIT {
        return Ok(None);
    }
    let cat = &d.catalog;
    let settings = d.store.get_settings();

    let gates = get_stage::<GatesStage>(d, id, STAGE_GATES)?
        .ok_or_else(|| PipelineError::Internal(format!("session {id} has no gates result")))?
        .gates;

    let decisions = current_decisions(d, id)?
        .into_iter()
        .map(|decision| {
            let dt = cat.type_by_id(&decision.type_id);
            let reviews = d.store.reviews_for_decision(&decision.id);
            let status = adr_status(&reviews);
            DecisionView {
                type_name: dt.map(|t| t.name.clone()).unwrap_or_else(|| decision.type_id.clone()),
                options: dt
                    .map(|t| {
                        t.options
                            .iter()
                            .map(|o| OptionView {
                                id: o.id.clone(),
                                name: o.name.clone(),
                                ring: o.ring,
                                description: o.description.clone(),
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                reviews,
                status,
                status_label: status.label().to_string(),
                decision,
            }
        })
        .collect();

    let not_applicable = get_stage::<ApplicabilityStage>(d, id, STAGE_APPLICABILITY)?
        .map(|a| a.not_applicable)
        .unwrap_or_default()
        .into_iter()
        .map(|na| NotApplicableView {
            type_name: cat
                .type_by_id(&na.type_id)
                .map(|t| t.name.clone())
                .unwrap_or_else(|| na.type_id.clone()),
            type_id: na.type_id,
            probability: na.probability,
        })
        .collect();

    let (input_tokens, cost_usd) = d.store.session_usage(id);
    let criteria = cat
        .criteria
        .iter()
        .map(|c| CriterionView {
            id: c.id.clone(),
            name: c.name.clone(),
            weight: settings.weights.get(&c.id).copied().unwrap_or(c.weight),
        })
        .collect();

    Ok(Some(Report {
        gates,
        decisions,
        not_applicable,
        input_tokens,
        cost_usd,
        truncated: stage_truncated(d, id)?,
        criteria,
    }))
}

/// Everything the UI shows for one session (IPC contract `SessionView`).
/// Pure replay from the store.
pub fn session_view(d: &Deps, id: &str) -> Result<SessionView, PipelineError> {
    let row = session_row(d, id)?;
    let brief_stage: Option<BriefStage> = get_stage(d, id, STAGE_BRIEF)?;
    let needs_reconfirm = brief_stage.as_ref().is_some_and(|s| s.needs_reconfirm);

    Ok(SessionView {
        sections: row_sections(&row)?,
        doc_path: brief_stage.and_then(|s| s.doc_path),
        brief: row_brief(&row)?,
        report: build_report(d, id)?,
        session: Session {
            brief_confirmed_at: if needs_reconfirm { None } else { row.brief_confirmed_at },
            id: row.id,
            created_at: row.created_at,
            title: row.title,
            mode: row.mode,
            input_text: row.input_text,
            doc_name: row.doc_name,
            stage: row.stage,
            error: row.error,
        },
    })
}

/// Every session, newest first, with decision counts (IPC contract
/// `SessionSummary`).
pub fn list_sessions(d: &Deps) -> Vec<SessionSummary> {
    d.store
        .list_sessions()
        .into_iter()
        .map(|row| {
            let decisions = current_decisions(d, &row.id).unwrap_or_default();
            SessionSummary {
                decision_count: decisions.len(),
                needs_architect_count: decisions.iter().filter(|dr| dr.route == Route::NeedsArchitect).count(),
                unreviewed_count: decisions
                    .iter()
                    .filter(|dr| d.store.reviews_for_decision(&dr.id).is_empty())
                    .count(),
                id: row.id,
                created_at: row.created_at,
                title: row.title,
                mode: row.mode,
                stage: row.stage,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::brief::BriefItem;

    fn brief(requirements: usize) -> Brief {
        Brief {
            summary: "A portal.".to_string(),
            context: ContextAssessment::default(),
            requirements: (0..requirements)
                .map(|i| BriefItem {
                    text: format!("requirement {i}"),
                    sources: vec![],
                })
                .collect(),
            nfrs: vec![BriefItem {
                text: "fast".to_string(),
                sources: vec![],
            }],
            constraints: vec![BriefItem {
                text: "tight budget".to_string(),
                sources: vec![],
            }],
            team_skills: vec![],
            mentioned_technologies: vec![],
        }
    }

    fn section(i: usize, chars: usize) -> Section {
        let text = "x".repeat(chars);
        Section {
            id: format!("S{i}"),
            heading: None,
            tokens: estimate_tokens(&text),
            text,
        }
    }

    fn tokens(v: &Value) -> usize {
        estimate_tokens(&serde_json::to_string(v).unwrap())
    }

    fn precedent_entries() -> Vec<Value> {
        (0..3)
            .map(|i| json!({ "option_id": "cosmos-db", "option_name": "Cosmos DB", "summary": format!("past project {i} {}", "p".repeat(400)) }))
            .collect()
    }

    #[test]
    fn fit_state_keeps_everything_when_under_budget() {
        let sections = vec![section(1, 100), section(2, 100)];
        let std = json!({ "type": "Document DB" });
        let f = fit_state(&brief(2), &sections, Some(&std), precedent_entries(), 100_000).unwrap();
        assert!(!f.truncated);
        assert_eq!(f.section_ids, vec!["S1", "S2"]);
        assert_eq!(f.state["bistec_precedent"].as_array().unwrap().len(), 3);
        assert_eq!(f.state["bistec_standard"], std);
    }

    #[test]
    fn fit_state_drops_precedent_before_any_evidence() {
        let sections = vec![section(1, 400), section(2, 400)];
        let full = fit_state(&brief(2), &sections, None, precedent_entries(), 100_000).unwrap();
        let without_precedent = fit_state(&brief(2), &sections, None, vec![], 100_000).unwrap();
        // A budget that fits everything except the precedent.
        let budget = tokens(&without_precedent.state);
        assert!(tokens(&full.state) > budget);

        let f = fit_state(&brief(2), &sections, None, precedent_entries(), budget).unwrap();
        assert!(f.state.get("bistec_precedent").is_none());
        assert_eq!(f.section_ids, vec!["S1", "S2"]);
        assert!(!f.truncated, "dropping precedent alone is not a truncation");
    }

    #[test]
    fn fit_state_then_drops_sections_from_the_end_then_requirements() {
        let sections = vec![section(1, 400), section(2, 400), section(3, 400)];
        let one_section = fit_state(&brief(3), &sections[..1], None, vec![], 100_000).unwrap();
        let f = fit_state(&brief(3), &sections, None, precedent_entries(), tokens(&one_section.state)).unwrap();
        assert!(f.truncated);
        assert_eq!(f.section_ids, vec!["S1"]);
        assert_eq!(f.state["brief"]["requirements"].as_array().unwrap().len(), 3);

        let no_sections_one_req = fit_state(&brief(1), &[], None, vec![], 100_000).unwrap();
        let f = fit_state(&brief(3), &sections, None, vec![], tokens(&no_sections_one_req.state)).unwrap();
        assert!(f.truncated);
        assert!(f.section_ids.is_empty());
        assert!(f.state.get("evidence_sections").is_none());
        let reqs = f.state["brief"]["requirements"].as_array().unwrap();
        assert_eq!(reqs.len(), 1);
        assert_eq!(reqs[0]["text"], "requirement 0");
        // constraints and NFRs are kept.
        assert_eq!(f.state["brief"]["constraints"].as_array().unwrap().len(), 1);
        assert_eq!(f.state["brief"]["nfrs"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn fit_state_errors_when_even_the_minimum_does_not_fit() {
        let err = fit_state(&brief(3), &[section(1, 400)], None, vec![], 10).unwrap_err();
        assert_eq!(err.code(), "validation");
    }

    #[test]
    fn in_stage_errors_keep_the_inner_code() {
        let e = PipelineError::Jev("boom".to_string()).in_stage("gates");
        assert_eq!(e.code(), "jev");
        assert_eq!(e.stage(), Some("gates"));
        assert_eq!(e.to_string(), "Jev call failed: boom");
        assert_eq!(PipelineError::DataNoticeRequired.stage(), None);
    }

    #[test]
    fn title_is_flattened_and_capped_at_60_chars() {
        assert_eq!(title_from("  short\n text "), "short text");
        let long = "a".repeat(100);
        let t = title_from(&long);
        assert_eq!(t.chars().count(), 61);
        assert!(t.ends_with('…'));
    }
}
