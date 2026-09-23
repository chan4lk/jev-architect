//! The Tauri command surface (IPC contract,
//! `.specclaw/changes/001-bistec-architect-agent/ipc-contract.md`).
//!
//! Every command's logic is a plain function over [`AppState`] and a
//! [`ClientFactory`], so it can be tested without a Tauri runtime; the
//! `#[tauri::command]` functions in [`ipc`] are thin wrappers that pass
//! [`LiveClients`] and forward progress as `pipeline://progress` events.
//!
//! The OpenRouter API key never leaves this process: no command returns it,
//! and `has_api_key` is the only thing the UI can learn about it (FR-2,
//! NFR-3, AC-3).

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use serde::Serialize;
use serde_json::json;

use crate::docs::{parse_document, total_tokens, DocError};
use crate::jev::{DecisionClient, DecisionRequest, DecisionResponse, HttpJev, JevError, Question};
use crate::model::brief::Brief;
use crate::model::catalog::Catalog;
use crate::model::report::{CriterionView, DecisionView};
use crate::model::review::{NewReview, ReviewAction};
use crate::model::settings::{Settings, DATA_NOTICE};
use crate::ollama::{pull_command, LocalModel, LocalModelError, OllamaModel};
use crate::pipeline::{self, Deps, PipelineError, ProgressSink, SessionSummary, SessionView};
use crate::render::{self, RenderContext, RenderError, ReportFormat};
use crate::secrets::{SecretError, SecretStore};
use crate::store::{Store, StoreError};

/// The Tauri-managed application state.
pub struct AppState {
    pub catalog: Arc<Catalog>,
    pub store: Arc<Store>,
    pub secrets: Arc<dyn SecretStore>,
}

// ---------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------

/// Every command's rejection (IPC contract `AppError`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AppError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
}

impl AppError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        AppError {
            code: code.to_string(),
            message: message.into(),
            stage: None,
        }
    }

    fn validation(message: impl Into<String>) -> Self {
        AppError::new("validation", message)
    }
}

impl From<PipelineError> for AppError {
    fn from(e: PipelineError) -> Self {
        AppError {
            code: e.code().to_string(),
            stage: e.stage().map(str::to_string),
            message: e.to_string(),
        }
    }
}

impl From<StoreError> for AppError {
    fn from(e: StoreError) -> Self {
        match e {
            StoreError::Review(e) => AppError::validation(e.to_string()),
            e => AppError::new("store", e.to_string()),
        }
    }
}

impl From<SecretError> for AppError {
    fn from(e: SecretError) -> Self {
        AppError::new("internal", e.to_string())
    }
}

impl From<RenderError> for AppError {
    fn from(e: RenderError) -> Self {
        match e {
            RenderError::Io(e) => AppError::new("io", e.to_string()),
            e => AppError::new("internal", e.to_string()),
        }
    }
}

impl From<DocError> for AppError {
    fn from(e: DocError) -> Self {
        PipelineError::from(e).into()
    }
}

pub type CmdResult<T> = Result<T, AppError>;

// ---------------------------------------------------------------------
// Model clients
// ---------------------------------------------------------------------

/// Builds the model clients for one command from the current settings.
pub trait ClientFactory: Send + Sync {
    /// `api_key` is `None` when no key is stored; the client must then fail
    /// with `JevError::NoApiKey` without touching the network.
    fn jev(&self, settings: &Settings, api_key: Option<String>) -> Arc<dyn DecisionClient>;
    fn local(&self, settings: &Settings) -> Arc<dyn LocalModel>;
}

/// The real clients: `HttpJev` (OpenRouter) and `OllamaModel`.
pub struct LiveClients;

impl ClientFactory for LiveClients {
    fn jev(&self, settings: &Settings, api_key: Option<String>) -> Arc<dyn DecisionClient> {
        match api_key {
            Some(key) => Arc::new(HttpJev::new(key).with_base_url(settings.jev_base_url.clone())),
            None => Arc::new(NoKeyJev),
        }
    }

    fn local(&self, settings: &Settings) -> Arc<dyn LocalModel> {
        Arc::new(OllamaModel::new(settings.ollama_base_url.clone(), settings.ollama_model.clone()))
    }
}

/// The Jev client used when no API key is stored: every call fails with
/// `no_api_key`, and nothing is sent.
pub struct NoKeyJev;

#[async_trait]
impl DecisionClient for NoKeyJev {
    async fn decide(&self, _req: &DecisionRequest) -> Result<DecisionResponse, JevError> {
        Err(JevError::NoApiKey)
    }
}

fn api_key(state: &AppState) -> CmdResult<Option<String>> {
    Ok(state.secrets.get()?)
}

fn deps(state: &AppState, clients: &dyn ClientFactory) -> CmdResult<Deps> {
    let settings = state.store.get_settings();
    Ok(Deps {
        catalog: state.catalog.clone(),
        store: state.store.clone(),
        jev: clients.jev(&settings, api_key(state)?),
        local: clients.local(&settings),
    })
}

/// Deps for a read-only replay (never calls a model).
fn replay_deps(state: &AppState) -> Deps {
    let settings = state.store.get_settings();
    Deps {
        catalog: state.catalog.clone(),
        store: state.store.clone(),
        jev: Arc::new(NoKeyJev),
        local: LiveClients.local(&settings),
    }
}

/// The gate in front of every command that sends user content to Jev
/// (FR-3, AC-17): the data notice must be acknowledged, and a key stored.
fn require_jev_ready(state: &AppState) -> CmdResult<()> {
    if !state.store.data_notice_acked() {
        return Err(PipelineError::DataNoticeRequired.into());
    }
    if !state.secrets.has()? {
        return Err(PipelineError::NoApiKey.into());
    }
    Ok(())
}

// ---------------------------------------------------------------------
// Settings, key, data notice
// ---------------------------------------------------------------------

/// `settings` with the catalogue's default weights filled in for every
/// criterion it doesn't override.
fn effective_settings(cat: &Catalog, mut settings: Settings) -> Settings {
    let mut weights = cat.criterion_weights();
    weights.append(&mut settings.weights);
    settings.weights = weights;
    settings
}

pub fn get_settings(state: &AppState) -> CmdResult<Settings> {
    Ok(effective_settings(&state.catalog, state.store.get_settings()))
}

pub fn save_settings(state: &AppState, settings: Settings) -> CmdResult<Settings> {
    state.store.save_settings(&settings)?;
    get_settings(state)
}

pub fn get_criteria(state: &AppState) -> CmdResult<Vec<CriterionView>> {
    Ok(state
        .catalog
        .criteria
        .iter()
        .map(|c| CriterionView {
            id: c.id.clone(),
            name: c.name.clone(),
            weight: c.weight,
        })
        .collect())
}

pub fn set_api_key(state: &AppState, key: &str) -> CmdResult<()> {
    let key = key.trim();
    if key.is_empty() {
        return Err(AppError::validation("The API key must not be empty"));
    }
    Ok(state.secrets.set(key)?)
}

pub fn clear_api_key(state: &AppState) -> CmdResult<()> {
    Ok(state.secrets.clear()?)
}

pub fn has_api_key(state: &AppState) -> CmdResult<bool> {
    Ok(state.secrets.has()?)
}

/// IPC contract `data_notice` result.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DataNotice {
    pub text: String,
    pub acked: bool,
}

pub fn data_notice(state: &AppState) -> CmdResult<DataNotice> {
    Ok(DataNotice {
        text: DATA_NOTICE.to_string(),
        acked: state.store.data_notice_acked(),
    })
}

pub fn ack_data_notice(state: &AppState) -> CmdResult<()> {
    Ok(state.store.ack_data_notice()?)
}

// ---------------------------------------------------------------------
// Health check (FR-4)
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenRouterStatus {
    Ok,
    NoKey,
    Error,
}

/// IPC contract `Health`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Health {
    pub ollama_reachable: bool,
    pub model_present: bool,
    pub model: String,
    pub pull_command: String,
    pub openrouter: OpenRouterStatus,
    pub openrouter_error: Option<String>,
}

/// The Ollama half of the health check: reachability and model presence,
/// with no Jev call. Shared by [`health_check`] and [`local_model_status`]
/// so there is exactly one implementation (spec edge case "Ollama not
/// running / model not pulled").
async fn probe_local_model(local: &dyn LocalModel) -> (bool, bool) {
    match local.model_present().await {
        Ok(present) => (true, present),
        Err(LocalModelError::Unreachable(_) | LocalModelError::Timeout) => (false, false),
        Err(_) => (true, false),
    }
}

/// IPC contract `LocalModelStatus`: a lightweight, Jev-free probe the UI can
/// call on every Home screen load to gate Mode A (Describe) without the cost
/// of a real Jev call (unlike [`health_check`], which is user-initiated).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LocalModelStatus {
    pub ollama_reachable: bool,
    pub model_present: bool,
    pub model: String,
    pub pull_command: String,
}

/// Reports whether the local model (Ollama) is reachable and pulled. Never
/// touches Jev/OpenRouter, so it's safe and cheap to call on every Home
/// screen load, unlike [`health_check`].
pub async fn local_model_status(state: &AppState, clients: &dyn ClientFactory) -> CmdResult<LocalModelStatus> {
    let settings = state.store.get_settings();
    let local = clients.local(&settings);
    let (ollama_reachable, model_present) = probe_local_model(local.as_ref()).await;
    Ok(LocalModelStatus {
        ollama_reachable,
        model_present,
        model: settings.ollama_model.clone(),
        pull_command: pull_command(&settings.ollama_model),
    })
}

/// Reports Ollama reachability, model presence, and whether the OpenRouter
/// key works (one minimal Noul call). User-initiated and sends no user
/// content, so it is exempt from the data-notice gate.
pub async fn health_check(state: &AppState, clients: &dyn ClientFactory) -> CmdResult<Health> {
    let settings = state.store.get_settings();
    let local = clients.local(&settings);
    let (ollama_reachable, model_present) = probe_local_model(local.as_ref()).await;

    let (openrouter, openrouter_error) = match api_key(state)? {
        None => (OpenRouterStatus::NoKey, None),
        Some(key) => {
            let jev = clients.jev(&settings, Some(key.clone()));
            let req = DecisionRequest {
                model: settings.jev_model.clone(),
                state: json!({ "check": "connection test" }),
                questions: [(
                    "health__ok".to_string(),
                    Question::noul("Is this a connection test?", "It is a connection test.", "It is not."),
                )]
                .into_iter()
                .collect(),
            };
            match jev.decide(&req).await {
                Ok(_) => (OpenRouterStatus::Ok, None),
                Err(e) => (OpenRouterStatus::Error, Some(e.to_string().replace(&key, "[REDACTED]"))),
            }
        }
    };

    Ok(Health {
        ollama_reachable,
        model_present,
        model: settings.ollama_model.clone(),
        pull_command: pull_command(&settings.ollama_model),
        openrouter,
        openrouter_error,
    })
}

// ---------------------------------------------------------------------
// Sessions and the pipeline
// ---------------------------------------------------------------------

pub async fn start_describe(state: &AppState, clients: &dyn ClientFactory, text: &str) -> CmdResult<SessionView> {
    let d = deps(state, clients)?;
    let id = pipeline::start_describe(&d, text).await?;
    Ok(pipeline::session_view(&d, &id)?)
}

pub async fn start_upload(
    state: &AppState,
    clients: &dyn ClientFactory,
    path: &str,
    sink: &dyn ProgressSink,
) -> CmdResult<SessionView> {
    let path = Path::new(path);
    // The small path sends the document to Jev before the review step.
    let small = total_tokens(&parse_document(path)?) <= state.store.get_settings().state_token_budget;
    if small {
        require_jev_ready(state)?;
    }
    let d = deps(state, clients)?;
    let id = pipeline::start_upload(&d, path, sink).await?;
    Ok(pipeline::session_view(&d, &id)?)
}

pub fn get_session(state: &AppState, id: &str) -> CmdResult<SessionView> {
    Ok(pipeline::session_view(&replay_deps(state), id)?)
}

pub fn list_sessions(state: &AppState) -> CmdResult<Vec<SessionSummary>> {
    Ok(pipeline::list_sessions(&replay_deps(state)))
}

pub fn update_brief(state: &AppState, id: &str, brief: &Brief) -> CmdResult<SessionView> {
    let d = replay_deps(state);
    pipeline::update_brief(&d, id, brief)?;
    Ok(pipeline::session_view(&d, id)?)
}

pub fn confirm_brief(state: &AppState, id: &str) -> CmdResult<SessionView> {
    let d = replay_deps(state);
    pipeline::confirm_brief(&d, id)?;
    Ok(pipeline::session_view(&d, id)?)
}

/// Runs (or, after a failure, resumes) the decision pass.
pub async fn run_decisions(
    state: &AppState,
    clients: &dyn ClientFactory,
    id: &str,
    sink: &dyn ProgressSink,
) -> CmdResult<SessionView> {
    require_jev_ready(state)?;
    let d = deps(state, clients)?;
    pipeline::run_decisions(&d, id, sink).await?;
    Ok(pipeline::session_view(&d, id)?)
}

/// Retry = run again: completed stages are skipped, so the pass resumes
/// from the failed stage (FR-15).
pub async fn retry(
    state: &AppState,
    clients: &dyn ClientFactory,
    id: &str,
    sink: &dyn ProgressSink,
) -> CmdResult<SessionView> {
    run_decisions(state, clients, id, sink).await
}

// ---------------------------------------------------------------------
// Review (FR-18)
// ---------------------------------------------------------------------

/// The id of the session that owns `decision_id`.
fn decision_session(store: &Store, decision_id: &str) -> CmdResult<String> {
    store
        .list_sessions()
        .into_iter()
        .find(|s| store.decisions_for_session(&s.id).iter().any(|d| d.id == decision_id))
        .map(|s| s.id)
        .ok_or_else(|| PipelineError::NotFound(format!("decision {decision_id}")).into())
}

fn find_view(d: &Deps, session_id: &str, decision_id: &str) -> CmdResult<DecisionView> {
    let report = pipeline::build_report(d, session_id)?
        .ok_or_else(|| AppError::validation("Decisions can be reviewed once the decision pass has finished"))?;
    report
        .decisions
        .into_iter()
        .find(|v| v.decision.id == decision_id)
        .ok_or_else(|| PipelineError::NotFound(format!("decision {decision_id} in the current report")).into())
}

pub fn review(
    state: &AppState,
    decision_id: &str,
    action: ReviewAction,
    option_id: Option<String>,
    reason: Option<String>,
) -> CmdResult<DecisionView> {
    let reviewer = state.store.get_settings().reviewer_name.trim().to_string();
    if reviewer.is_empty() {
        return Err(AppError::validation(
            "Set your reviewer name in Settings before approving decisions",
        ));
    }
    let d = replay_deps(state);
    let session_id = decision_session(&state.store, decision_id)?;
    let view = find_view(&d, &session_id, decision_id)?;
    if let Some(option) = option_id.as_deref().filter(|o| !o.trim().is_empty()) {
        if !view.options.iter().any(|o| o.id == option) {
            return Err(AppError::validation(format!(
                "'{option}' is not an option of {}",
                view.type_name
            )));
        }
    }

    state.store.append_review(&NewReview {
        decision_id: decision_id.to_string(),
        action,
        option_id,
        reviewer,
        reason,
    })?;
    find_view(&d, &session_id, decision_id)
}

// ---------------------------------------------------------------------
// Export (FR-19, FR-20)
// ---------------------------------------------------------------------

/// Renders with a context built from the session's finished report.
fn with_render_ctx<T>(
    state: &AppState,
    id: &str,
    f: impl FnOnce(&RenderContext) -> CmdResult<T>,
) -> CmdResult<T> {
    let view = pipeline::session_view(&replay_deps(state), id)?;
    let (Some(brief), Some(report)) = (view.brief.as_ref(), view.report.as_ref()) else {
        return Err(AppError::validation("The decision pass has not finished for this session"));
    };
    let cloud_choice = report
        .decisions
        .iter()
        .find(|v| v.decision.type_id == "cloud-platform")
        .and_then(|v| v.options.iter().find(|o| o.id == v.decision.choice))
        .map(|o| o.name.as_str());
    let ctx = RenderContext {
        title: &view.session.title,
        brief,
        report,
        cloud_choice,
        today: chrono::Local::now().date_naive(),
    };
    f(&ctx)
}

/// `NNN` from a path named `ADR-NNN-<slug>.md`.
fn adr_number(path: &Path) -> Option<u32> {
    path.file_name()?.to_str()?.strip_prefix("ADR-")?.split('-').next()?.parse().ok()
}

pub fn export_adrs(state: &AppState, id: &str, dir: &str) -> CmdResult<Vec<String>> {
    with_render_ctx(state, id, |ctx| {
        let paths = render::export_adrs(&state.catalog, ctx, Path::new(dir))?;
        for (view, path) in ctx.report.decisions.iter().zip(&paths) {
            let n = adr_number(path)
                .ok_or_else(|| AppError::new("internal", format!("unexpected ADR file name {}", path.display())))?;
            state.store.set_adr_number(&view.decision.id, n)?;
        }
        Ok(paths.iter().map(|p| p.display().to_string()).collect())
    })
}

pub fn export_report(state: &AppState, id: &str, dir: &str, format: &str) -> CmdResult<String> {
    let format = match format {
        "md" => ReportFormat::Md,
        "html" => ReportFormat::Html,
        other => return Err(AppError::validation(format!("unknown report format '{other}' (md or html)"))),
    };
    with_render_ctx(state, id, |ctx| {
        let path = render::export_report(&state.catalog, ctx, Path::new(dir), format)?;
        Ok(path.display().to_string())
    })
}

// ---------------------------------------------------------------------
// Tauri wrappers
// ---------------------------------------------------------------------

/// The `#[tauri::command]` functions registered in `lib.rs`. Each one only
/// unwraps Tauri's arguments and calls the plain function above.
pub mod ipc {
    use tauri::{AppHandle, Emitter, State};

    use super::*;
    use crate::pipeline::Progress;

    /// Forwards pipeline progress as `pipeline://progress` events.
    struct EventSink(AppHandle);

    impl ProgressSink for EventSink {
        fn emit(&self, p: Progress) {
            if let Err(e) = self.0.emit("pipeline://progress", p) {
                log::warn!("could not emit progress event: {e}");
            }
        }
    }

    type S<'a> = State<'a, AppState>;

    #[tauri::command]
    pub async fn get_settings(state: S<'_>) -> CmdResult<Settings> {
        super::get_settings(&state)
    }

    #[tauri::command]
    pub async fn save_settings(state: S<'_>, settings: Settings) -> CmdResult<Settings> {
        super::save_settings(&state, settings)
    }

    #[tauri::command]
    pub async fn get_criteria(state: S<'_>) -> CmdResult<Vec<CriterionView>> {
        super::get_criteria(&state)
    }

    #[tauri::command]
    pub async fn set_api_key(state: S<'_>, key: String) -> CmdResult<()> {
        super::set_api_key(&state, &key)
    }

    #[tauri::command]
    pub async fn clear_api_key(state: S<'_>) -> CmdResult<()> {
        super::clear_api_key(&state)
    }

    #[tauri::command]
    pub async fn has_api_key(state: S<'_>) -> CmdResult<bool> {
        super::has_api_key(&state)
    }

    #[tauri::command]
    pub async fn data_notice(state: S<'_>) -> CmdResult<DataNotice> {
        super::data_notice(&state)
    }

    #[tauri::command]
    pub async fn ack_data_notice(state: S<'_>) -> CmdResult<()> {
        super::ack_data_notice(&state)
    }

    #[tauri::command]
    pub async fn health_check(state: S<'_>) -> CmdResult<Health> {
        super::health_check(&state, &LiveClients).await
    }

    #[tauri::command]
    pub async fn local_model_status(state: S<'_>) -> CmdResult<LocalModelStatus> {
        super::local_model_status(&state, &LiveClients).await
    }

    #[tauri::command]
    pub async fn start_describe(state: S<'_>, text: String) -> CmdResult<SessionView> {
        super::start_describe(&state, &LiveClients, &text).await
    }

    #[tauri::command]
    pub async fn start_upload(app: AppHandle, state: S<'_>, path: String) -> CmdResult<SessionView> {
        super::start_upload(&state, &LiveClients, &path, &EventSink(app)).await
    }

    #[tauri::command]
    pub async fn get_session(state: S<'_>, id: String) -> CmdResult<SessionView> {
        super::get_session(&state, &id)
    }

    #[tauri::command]
    pub async fn list_sessions(state: S<'_>) -> CmdResult<Vec<SessionSummary>> {
        super::list_sessions(&state)
    }

    #[tauri::command]
    pub async fn update_brief(state: S<'_>, id: String, brief: Brief) -> CmdResult<SessionView> {
        super::update_brief(&state, &id, &brief)
    }

    #[tauri::command]
    pub async fn confirm_brief(state: S<'_>, id: String) -> CmdResult<SessionView> {
        super::confirm_brief(&state, &id)
    }

    #[tauri::command]
    pub async fn run_decisions(app: AppHandle, state: S<'_>, id: String) -> CmdResult<SessionView> {
        super::run_decisions(&state, &LiveClients, &id, &EventSink(app)).await
    }

    #[tauri::command]
    pub async fn retry(app: AppHandle, state: S<'_>, id: String) -> CmdResult<SessionView> {
        super::retry(&state, &LiveClients, &id, &EventSink(app)).await
    }

    #[tauri::command]
    pub async fn review(
        state: S<'_>,
        decision_id: String,
        action: ReviewAction,
        option_id: Option<String>,
        reason: Option<String>,
    ) -> CmdResult<DecisionView> {
        super::review(&state, &decision_id, action, option_id, reason)
    }

    #[tauri::command]
    pub async fn export_adrs(state: S<'_>, id: String, dir: String) -> CmdResult<Vec<String>> {
        super::export_adrs(&state, &id, &dir)
    }

    #[tauri::command]
    pub async fn export_report(state: S<'_>, id: String, dir: String, format: String) -> CmdResult<String> {
        super::export_report(&state, &id, &dir, &format)
    }
}
