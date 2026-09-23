//! AC-3 / NFR-3: the OpenRouter API key never appears in *log output*.
//!
//! `commands.rs:api_key_is_stored_but_never_returned_by_any_command` already
//! proves the key never leaks through IPC results, and
//! `store.rs:the_sqlite_file_never_contains_the_api_key` proves it never
//! leaks into the on-disk database. This test closes the third half: it
//! installs a real `log::Log` implementation (the crate uses the `log`
//! facade; `tauri-plugin-log` is just one consumer of it), runs a full fake
//! Mode A flow through the command-layer functions with a
//! deliberately-obvious key, and greps every captured record for it. It also
//! greps the `Debug` formatting of a real `HttpJev` and of a `JevError`
//! surfaced from an actual HTTP round trip (wiremock), since those are the
//! two places a stray `log::debug!("{:?}", …)` could leak the key.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};

use bistec_architect::commands::{self as cmd, AppState, ClientFactory};
use bistec_architect::jev::{
    Answer, DecisionClient, DecisionRequest, DecisionResponse, FakeJev, HttpJev, JevError, Question, Usage,
};
use bistec_architect::model::catalog::Catalog;
use bistec_architect::model::settings::Settings;
use bistec_architect::ollama::{FakeLocalModel, LocalModel};
use bistec_architect::pipeline::NoopSink;
use bistec_architect::secrets::MemoryStore;
use bistec_architect::store::Store;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The key substring under test: distinctive enough that a match can only
/// come from a real leak, never a coincidence.
const KEY: &str = "sk-or-test-SECRET-LOGGREP";
const DESCRIPTION: &str = "We need a customer self-service portal for an insurer, built by a .NET team on Azure.";

// ---- a capturing `log::Log` -------------------------------------------------

struct CapturingLogger;

fn log_buf() -> &'static Mutex<String> {
    static BUF: OnceLock<Mutex<String>> = OnceLock::new();
    BUF.get_or_init(|| Mutex::new(String::new()))
}

impl log::Log for CapturingLogger {
    fn enabled(&self, _metadata: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        let mut buf = log_buf().lock().expect("log buffer mutex poisoned");
        buf.push_str(&format!("[{}] {}\n", record.level(), record.args()));
    }

    fn flush(&self) {}
}

/// Installs the capturing logger exactly once per test binary (the `log`
/// crate only allows one global logger per process).
fn install_capturing_logger() {
    static INSTALLED: OnceLock<()> = OnceLock::new();
    INSTALLED.get_or_init(|| {
        log::set_boxed_logger(Box::new(CapturingLogger)).expect("install capturing logger");
        log::set_max_level(log::LevelFilter::Trace);
    });
}

fn captured_log() -> String {
    log_buf().lock().expect("log buffer mutex poisoned").clone()
}

// ---- fakes for the command-layer flow --------------------------------------

type Handler = Box<dyn Fn(&DecisionRequest) -> Result<DecisionResponse, JevError> + Send + Sync>;

fn answer(req: &DecisionRequest) -> DecisionResponse {
    let answers = req
        .questions
        .iter()
        .map(|(key, q)| {
            let a = match q {
                Question::Noul { .. } => Answer::Noul {
                    noul: if key.starts_with("gate__injection") || key.starts_with("context__compliance__") {
                        0.0
                    } else {
                        0.9
                    },
                },
                Question::Choice { criteria, .. } => {
                    let chosen = criteria.keys().next().unwrap().clone();
                    let others = (criteria.len().max(2) - 1) as f64;
                    Answer::Choice {
                        probabilities: criteria
                            .keys()
                            .map(|k| (k.clone(), if *k == chosen { 0.8 } else { 0.2 / others }))
                            .collect(),
                        choice: chosen,
                        confidence: 0.8,
                    }
                }
                Question::Score { .. } => Answer::Score {
                    score: 3.0,
                    confidence: 0.9,
                    probabilities: BTreeMap::new(),
                    legend: None,
                },
            };
            (key.clone(), a)
        })
        .collect();
    DecisionResponse {
        id: None,
        model: "typesafe/jev-1.13-20260901".to_string(),
        provider: None,
        answers,
        usage: Usage {
            input_tokens: 100,
            output_tokens: 10,
            cost: Some(0.001),
        },
    }
}

fn brief_json() -> String {
    json!({
        "summary": "A customer self-service portal for an insurer.",
        "context": {
            "scale": "medium", "budget": "moderate", "timeline": "normal",
            "team_size": "small", "compliance": ["gdpr"], "data_sensitivity": "confidential"
        },
        "requirements": [{"text": "Policyholders track claims", "sources": []}],
        "nfrs": [{"text": "Pages load in under two seconds", "sources": []}],
        "constraints": [],
        "team_skills": [{"text": "The team knows .NET", "sources": []}],
        "mentioned_technologies": [".NET", "Azure"]
    })
    .to_string()
}

struct FakeClients {
    jev: Arc<FakeJev<Handler>>,
    local: Arc<FakeLocalModel>,
}

impl ClientFactory for FakeClients {
    fn jev(&self, _settings: &Settings, _api_key: Option<String>) -> Arc<dyn DecisionClient> {
        self.jev.clone()
    }

    fn local(&self, _settings: &Settings) -> Arc<dyn LocalModel> {
        self.local.clone()
    }
}

fn fake_clients() -> FakeClients {
    FakeClients {
        jev: Arc::new(FakeJev::new(Box::new(|req: &DecisionRequest| Ok(answer(req))) as Handler)),
        local: Arc::new(FakeLocalModel::new(
            "hf.co/openbmb/MiniCPM5-2B-GGUF:Q4_K_M",
            true,
            vec![Ok(brief_json())],
        )),
    }
}

fn app_state() -> AppState {
    AppState {
        catalog: Arc::new(Catalog::bundled().unwrap()),
        store: Arc::new(Store::open_in_memory().unwrap()),
        secrets: Arc::new(MemoryStore::default()),
    }
}

/// A real HTTP request/response round trip against a mock OpenRouter server,
/// to exercise `jev.rs`'s own `log::debug!` call site and a real
/// `JevError` — the two paths where a stray key could leak into a log line.
async fn round_trip_through_real_http_jev() -> Result<(), JevError> {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/alpha/decisions"))
        .respond_with(ResponseTemplate::new(500).set_body_string("scripted failure"))
        .mount(&server)
        .await;

    let jev = HttpJev::new(KEY.to_string())
        .with_base_url(server.uri())
        .with_max_retries(0);
    // The Debug impl is what NFR-3 relies on to keep the key out of any
    // `log::debug!("{jev:?}")` call site; assert on it directly too.
    let debug_str = format!("{jev:?}");
    assert!(!debug_str.contains(KEY));
    log::debug!("constructed client: {debug_str}");

    let req = DecisionRequest {
        model: "typesafe/jev-1.13".to_string(),
        state: json!({ "brief": "irrelevant for this test" }),
        questions: BTreeMap::new(),
    };
    let err = jev.decide(&req).await.unwrap_err();
    // The error is exactly what a caller might log; it must not carry the key.
    log::debug!("jev call failed: {err:?}");
    Err(err)
}

#[tokio::test]
async fn no_log_record_ever_contains_the_api_key() {
    install_capturing_logger();

    // ---- the full fake Mode A flow, through the command layer -------------
    let state = app_state();
    let clients = fake_clients();
    cmd::set_api_key(&state, KEY).unwrap();
    cmd::ack_data_notice(&state).unwrap();

    let view = cmd::start_describe(&state, &clients, DESCRIPTION).await.unwrap();
    let id = view.session.id.clone();
    cmd::confirm_brief(&state, &id).unwrap();
    let done = cmd::run_decisions(&state, &clients, &id, &NoopSink).await.unwrap();
    assert_eq!(done.session.stage, "done");

    let dir = tempfile::tempdir().unwrap();
    cmd::export_adrs(&state, &id, dir.path().to_str().unwrap()).unwrap();
    cmd::export_report(&state, &id, dir.path().to_str().unwrap(), "md").unwrap();

    // ---- a real HttpJev round trip (Debug + JevError formatting) ----------
    assert!(round_trip_through_real_http_jev().await.is_err());

    let log = captured_log();
    assert!(!log.is_empty(), "the flow above should have produced at least one log record");
    assert!(
        !log.contains(KEY),
        "the API key leaked into the log output:\n{log}"
    );
    // Sanity: the meaningful jev.rs log line is actually the thing we captured.
    assert!(log.contains("Jev request:"), "expected the request-send debug line in:\n{log}");
    assert!(log.contains("[REDACTED]") || !log.contains("api_key"), "HttpJev's Debug impl should redact the key");
}
