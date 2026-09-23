//! Integration tests for `bistec_architect::jev`, against a mock HTTP server
//! (wiremock) standing in for the OpenRouter Decisions API.

use std::collections::BTreeMap;
use std::time::Duration;

use bistec_architect::jev::{DecisionClient, DecisionRequest, HttpJev, JevError, Question};
use serde_json::{json, Value};
use wiremock::matchers::{body_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn sample_request() -> DecisionRequest {
    let mut questions = BTreeMap::new();
    questions.insert(
        "is_bug".to_string(),
        Question::noul(
            Value::String("Is this report describing a bug?".to_string()),
            "Yes, this is a bug",
            "No, this is not a bug",
        ),
    );

    let mut team_criteria = BTreeMap::new();
    team_criteria.insert("payments".to_string(), Some(json!("Payments team")));
    team_criteria.insert("frontend".to_string(), Some(json!("Frontend team")));
    team_criteria.insert("account".to_string(), Some(json!("Account team")));
    questions.insert(
        "team".to_string(),
        Question::choice(Value::String("Which team owns this?".to_string()), team_criteria),
    );

    questions.insert(
        "urgency".to_string(),
        Question::score(
            Value::String("How urgent is this?".to_string()),
            vec![
                json!("Can wait for the next release"),
                json!("Should be fixed this week"),
                json!("Blocking revenue right now"),
            ],
        ),
    );

    DecisionRequest {
        model: "typesafe/jev-1.13".to_string(),
        state: json!({ "report": "checkout is charging customers twice" }),
        questions,
    }
}

fn fixture(name: &str) -> Value {
    let path = format!("{}/tests/fixtures/jev/{}", env!("CARGO_MANIFEST_DIR"), name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading fixture {path}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parsing fixture {path}: {e}"))
}

#[tokio::test]
async fn sends_expected_request_body_and_auth_header() {
    let server = MockServer::start().await;
    let req = sample_request();

    let expected_body = json!({
        "model": "typesafe/jev-1.13",
        "state": { "report": "checkout is charging customers twice" },
        "questions": {
            "is_bug": {
                "type": "noul",
                "instructions": "Is this report describing a bug?",
                "criteria": { "true": "Yes, this is a bug", "false": "No, this is not a bug" }
            },
            "team": {
                "type": "choice",
                "instructions": "Which team owns this?",
                "criteria": {
                    "payments": "Payments team",
                    "frontend": "Frontend team",
                    "account": "Account team"
                }
            },
            "urgency": {
                "type": "score",
                "instructions": "How urgent is this?",
                "criteria": [
                    "Can wait for the next release",
                    "Should be fixed this week",
                    "Blocking revenue right now"
                ]
            }
        }
    });

    Mock::given(method("POST"))
        .and(path("/alpha/decisions"))
        .and(header("Authorization", "Bearer test-key-123"))
        .and(header("Content-Type", "application/json"))
        .and(body_json(expected_body))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("decision_response.json")))
        .expect(1)
        .mount(&server)
        .await;

    let client = HttpJev::new("test-key-123".to_string()).with_base_url(server.uri());
    let resp = client.decide(&req).await.expect("decide should succeed");

    assert_eq!(resp.model, "typesafe/jev-1.13-20260917");
    assert_eq!(resp.provider.as_deref(), Some("TypeSafe"));

    let is_bug = resp.answers.get("is_bug").expect("is_bug answer");
    assert_eq!(is_bug.as_noul(), Some(0.96));

    let team = resp.answers.get("team").expect("team answer");
    assert_eq!(team.as_choice(), Some("payments"));

    let urgency = resp.answers.get("urgency").expect("urgency answer");
    assert_eq!(urgency.as_score(), Some(1.99));

    assert_eq!(resp.usage.input_tokens, 476);
    assert_eq!(resp.usage.output_tokens, 70);
    assert_eq!(resp.usage.cost, Some(0.000019992));
}

#[tokio::test]
async fn usage_without_cost_parses_to_none() {
    let server = MockServer::start().await;
    let req = sample_request();

    Mock::given(method("POST"))
        .and(path("/alpha/decisions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("decision_response_no_cost.json")))
        .mount(&server)
        .await;

    // Only ask the one question the no-cost fixture answers, so keys line up.
    let mut questions = BTreeMap::new();
    questions.insert(
        "is_bug".to_string(),
        Question::noul(Value::String("Is this a bug?".to_string()), "yes", "no"),
    );
    let req = DecisionRequest { questions, ..req };

    let client = HttpJev::new("test-key-123".to_string()).with_base_url(server.uri());
    let resp = client.decide(&req).await.expect("decide should succeed");

    assert_eq!(resp.usage.cost, None);
}

#[tokio::test]
async fn retries_after_429_then_succeeds() {
    let server = MockServer::start().await;
    let req = sample_request();

    Mock::given(method("POST"))
        .and(path("/alpha/decisions"))
        .respond_with(ResponseTemplate::new(429))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/alpha/decisions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("decision_response.json")))
        .expect(1)
        .mount(&server)
        .await;

    let client = HttpJev::new("test-key-123".to_string())
        .with_base_url(server.uri())
        .with_backoff_base(Duration::from_millis(1));

    let resp = client.decide(&req).await.expect("decide should succeed after one retry");
    assert_eq!(resp.model, "typesafe/jev-1.13-20260917");

    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn gives_up_after_max_retries_with_typed_error() {
    let server = MockServer::start().await;
    let req = sample_request();

    Mock::given(method("POST"))
        .and(path("/alpha/decisions"))
        .respond_with(ResponseTemplate::new(500))
        .expect(4)
        .mount(&server)
        .await;

    let client = HttpJev::new("test-key-123".to_string())
        .with_base_url(server.uri())
        .with_max_retries(3)
        .with_backoff_base(Duration::from_millis(1));

    let err = client.decide(&req).await.expect_err("should exhaust retries");
    match err {
        JevError::RetriesExhausted { attempts, last } => {
            assert_eq!(attempts, 4);
            assert!(matches!(*last, JevError::Http { status: 500, .. }));
        }
        other => panic!("expected RetriesExhausted, got {other:?}"),
    }

    assert_eq!(server.received_requests().await.unwrap().len(), 4);
}

#[tokio::test]
async fn does_not_retry_other_4xx() {
    let server = MockServer::start().await;
    let req = sample_request();

    Mock::given(method("POST"))
        .and(path("/alpha/decisions"))
        .respond_with(ResponseTemplate::new(400).set_body_string("bad request"))
        .expect(1)
        .mount(&server)
        .await;

    let client = HttpJev::new("test-key-123".to_string())
        .with_base_url(server.uri())
        .with_backoff_base(Duration::from_millis(1));

    let err = client.decide(&req).await.expect_err("should fail without retrying");
    match err {
        JevError::Http { status, .. } => assert_eq!(status, 400),
        other => panic!("expected Http, got {other:?}"),
    }

    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn answer_key_mismatch_is_a_typed_error() {
    let server = MockServer::start().await;
    let req = sample_request();

    // The fixture only answers "is_bug", but the request asks "is_bug", "team",
    // and "urgency" — "team" and "urgency" are missing.
    Mock::given(method("POST"))
        .and(path("/alpha/decisions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture("decision_response_no_cost.json")))
        .mount(&server)
        .await;

    let client = HttpJev::new("test-key-123".to_string()).with_base_url(server.uri());
    let err = client.decide(&req).await.expect_err("should fail on key mismatch");

    match err {
        JevError::AnswerKeysMismatch { missing, unexpected } => {
            assert_eq!(missing, vec!["team".to_string(), "urgency".to_string()]);
            assert!(unexpected.is_empty());
        }
        other => panic!("expected AnswerKeysMismatch, got {other:?}"),
    }
}

#[tokio::test]
async fn debug_output_never_contains_the_api_key() {
    let client = HttpJev::new("super-secret-key-do-not-log".to_string());
    let debug_str = format!("{client:?}");
    assert!(!debug_str.contains("super-secret-key-do-not-log"));
}
