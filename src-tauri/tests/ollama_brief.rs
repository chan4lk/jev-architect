//! Integration tests for the Ollama client (T4): request shape, model
//! presence checks, and HTTP error handling, against a mock HTTP server.

use bistec_architect::ollama::{LocalModel, LocalModelError, OllamaModel};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn chat_request_has_format_stream_false_and_temperature_zero() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "message": {"role": "assistant", "content": "{\"ok\": true}"}
        })))
        .mount(&server)
        .await;

    let model = OllamaModel::new(server.uri(), "test-model");
    let schema = json!({"type": "object", "properties": {"ok": {"type": "boolean"}}});
    let result = model.chat_json("system prompt", "user text", &schema).await;
    assert_eq!(result.unwrap(), "{\"ok\": true}");

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    let body: serde_json::Value = requests[0].body_json().unwrap();

    assert_eq!(body["model"], json!("test-model"));
    assert_eq!(body["stream"], json!(false));
    assert_eq!(body["options"]["temperature"], json!(0));
    assert_eq!(body["format"], schema);
    assert_eq!(body["messages"][0]["role"], json!("system"));
    assert_eq!(body["messages"][0]["content"], json!("system prompt"));
    assert_eq!(body["messages"][1]["role"], json!("user"));
    assert_eq!(body["messages"][1]["content"], json!("user text"));
}

#[tokio::test]
async fn chat_request_500_is_reported_as_http_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(ResponseTemplate::new(500).set_body_string("internal error"))
        .mount(&server)
        .await;

    let model = OllamaModel::new(server.uri(), "test-model");
    let schema = json!({});
    let err = model.chat_json("s", "u", &schema).await.unwrap_err();
    match err {
        LocalModelError::Http { status, body } => {
            assert_eq!(status, 500);
            assert_eq!(body, "internal error");
        }
        other => panic!("expected Http error, got {other:?}"),
    }
}

#[tokio::test]
async fn model_present_true_when_tags_list_includes_configured_model() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/tags"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "models": [{"name": "llama3:latest"}, {"name": "other:1.0"}]
        })))
        .mount(&server)
        .await;

    let model = OllamaModel::new(server.uri(), "llama3");
    assert!(model.model_present().await.unwrap());
}

#[tokio::test]
async fn model_present_true_when_configured_name_carries_explicit_latest() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/tags"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "models": [{"name": "llama3"}]
        })))
        .mount(&server)
        .await;

    let model = OllamaModel::new(server.uri(), "llama3:latest");
    assert!(model.model_present().await.unwrap());
}

#[tokio::test]
async fn model_present_false_when_tags_list_does_not_include_configured_model() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/tags"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "models": [{"name": "other:1.0"}]
        })))
        .mount(&server)
        .await;

    let model = OllamaModel::new(server.uri(), "llama3");
    assert!(!model.model_present().await.unwrap());
}

/// Manual check only (not run in CI): a real extraction against a locally
/// running Ollama with the default model pulled. Run with
/// `cargo test --manifest-path src-tauri/Cargo.toml -- --ignored manual_real_extraction`.
#[tokio::test]
#[ignore = "manual: requires a running Ollama with the default model pulled"]
async fn manual_real_extraction_against_local_ollama() {
    use bistec_architect::brief::extract_brief_mode_a;
    use bistec_architect::ollama::{DEFAULT_BASE_URL, DEFAULT_MODEL};

    let model = OllamaModel::new(DEFAULT_BASE_URL, DEFAULT_MODEL);
    assert!(
        model.model_present().await.unwrap(),
        "default model is not pulled: run `ollama pull {DEFAULT_MODEL}`"
    );

    let text = "We need a small internal reporting tool for a team of 4. Budget is tight, \
                under $500/month. Must ship in 3 weeks. The team knows PostgreSQL and React. \
                No compliance requirements were mentioned.";
    let brief = extract_brief_mode_a(&model, text).await.unwrap();
    println!("extracted brief: {brief:#?}");
    assert!(!brief.summary.trim().is_empty());
}

#[tokio::test]
async fn tags_500_is_reported_as_http_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/tags"))
        .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
        .mount(&server)
        .await;

    let model = OllamaModel::new(server.uri(), "llama3");
    let err = model.model_present().await.unwrap_err();
    assert!(matches!(err, LocalModelError::Http { status: 500, .. }));
}
