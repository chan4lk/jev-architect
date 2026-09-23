//! Jev client: `DecisionClient` trait + `HttpJev` (reqwest, retries) over the
//! OpenRouter Decisions API (`POST {base_url}/alpha/decisions`), plus the
//! `Question` / `Answer` / `DecisionRequest` / `DecisionResponse` DTOs and a
//! `FakeJev` test double for other modules.
//!
//! See `.specclaw/changes/001-bistec-architect-agent/design.md` for the
//! request/response shapes (Jev request shapes, jaggedness guidance).

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Default OpenRouter API base URL (the Decisions API lives under `/alpha/decisions`).
const DEFAULT_BASE_URL: &str = "https://openrouter.ai/api";
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);
const DEFAULT_MAX_RETRIES: u32 = 3;
const DEFAULT_BACKOFF_BASE: Duration = Duration::from_millis(500);

/// A criteria pair for a `Noul` (yes/no) question.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NoulCriteria {
    #[serde(rename = "true")]
    pub yes: Value,
    #[serde(rename = "false")]
    pub no: Value,
}

/// A typed question sent to Jev in a `DecisionRequest`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Question {
    Noul {
        instructions: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        criteria: Option<NoulCriteria>,
    },
    Choice {
        instructions: Value,
        criteria: BTreeMap<String, Option<Value>>,
    },
    Score {
        instructions: Value,
        criteria: Vec<Value>,
    },
}

impl Question {
    /// Convenience constructor for a `Noul` question with explicit true/false criteria.
    pub fn noul(instructions: impl Into<Value>, yes: impl Into<Value>, no: impl Into<Value>) -> Self {
        Question::Noul {
            instructions: instructions.into(),
            criteria: Some(NoulCriteria {
                yes: yes.into(),
                no: no.into(),
            }),
        }
    }

    /// Convenience constructor for a `Choice` question.
    pub fn choice(instructions: impl Into<Value>, criteria: BTreeMap<String, Option<Value>>) -> Self {
        Question::Choice {
            instructions: instructions.into(),
            criteria,
        }
    }

    /// Convenience constructor for a `Score` question (one level description per entry).
    pub fn score(instructions: impl Into<Value>, levels: Vec<Value>) -> Self {
        Question::Score {
            instructions: instructions.into(),
            criteria: levels,
        }
    }
}

/// A typed answer returned by Jev in a `DecisionResponse`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Answer {
    Noul {
        noul: f64,
    },
    Choice {
        choice: String,
        confidence: f64,
        probabilities: BTreeMap<String, f64>,
    },
    Score {
        score: f64,
        confidence: f64,
        probabilities: BTreeMap<String, f64>,
        #[serde(default)]
        legend: Option<BTreeMap<String, String>>,
    },
}

impl Answer {
    pub fn as_noul(&self) -> Option<f64> {
        match self {
            Answer::Noul { noul } => Some(*noul),
            _ => None,
        }
    }

    pub fn as_choice(&self) -> Option<&str> {
        match self {
            Answer::Choice { choice, .. } => Some(choice.as_str()),
            _ => None,
        }
    }

    pub fn as_score(&self) -> Option<f64> {
        match self {
            Answer::Score { score, .. } => Some(*score),
            _ => None,
        }
    }
}

/// Token/cost usage reported for a single Jev call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(default)]
    pub cost: Option<f64>,
}

/// The request body for `POST {base_url}/alpha/decisions`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionRequest {
    pub model: String,
    pub state: Value,
    pub questions: BTreeMap<String, Question>,
}

/// The response body for `POST {base_url}/alpha/decisions`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionResponse {
    #[serde(default)]
    pub id: Option<String>,
    pub model: String,
    #[serde(default)]
    pub provider: Option<String>,
    pub answers: BTreeMap<String, Answer>,
    pub usage: Usage,
}

/// Errors from a Jev decision call.
#[derive(Debug, thiserror::Error)]
pub enum JevError {
    #[error("Jev HTTP error {status}: {body}")]
    Http { status: u16, body: String },
    #[error("Jev request timed out")]
    Timeout,
    #[error("Jev network error: {0}")]
    Network(String),
    #[error("failed to decode Jev response: {0}")]
    Decode(String),
    #[error("Jev answer keys did not match the questions asked (missing: {missing:?}, unexpected: {unexpected:?})")]
    AnswerKeysMismatch {
        missing: Vec<String>,
        unexpected: Vec<String>,
    },
    #[error("Jev retries exhausted after {attempts} attempts: {last}")]
    RetriesExhausted { attempts: u32, last: Box<JevError> },
    #[error("no OpenRouter API key is configured")]
    NoApiKey,
}

impl JevError {
    /// Whether this error is worth retrying (429, 5xx, timeouts, connection errors).
    fn is_retryable(&self) -> bool {
        match self {
            JevError::Http { status, .. } => *status == 429 || (500..600).contains(status),
            JevError::Timeout => true,
            JevError::Network(_) => true,
            JevError::Decode(_) => false,
            JevError::AnswerKeysMismatch { .. } => false,
            JevError::RetriesExhausted { .. } => false,
            JevError::NoApiKey => false,
        }
    }
}

/// Implemented by anything that can answer a `DecisionRequest` (the live HTTP
/// client, or a fake for tests).
#[async_trait]
pub trait DecisionClient: Send + Sync {
    async fn decide(&self, req: &DecisionRequest) -> Result<DecisionResponse, JevError>;
}

/// Live Jev client over the OpenRouter Decisions API.
///
/// `Debug` redacts the API key (NFR-3 / AC-4): it never appears in logs.
pub struct HttpJev {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
    timeout: Duration,
    max_retries: u32,
    backoff_base: Duration,
}

impl HttpJev {
    pub fn new(api_key: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key,
            base_url: DEFAULT_BASE_URL.to_string(),
            timeout: DEFAULT_TIMEOUT,
            max_retries: DEFAULT_MAX_RETRIES,
            backoff_base: DEFAULT_BACKOFF_BASE,
        }
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = max_retries;
        self
    }

    pub fn with_backoff_base(mut self, backoff_base: Duration) -> Self {
        self.backoff_base = backoff_base;
        self
    }

    /// Sends the request once and parses the response; does not retry.
    async fn try_once(&self, req: &DecisionRequest) -> Result<DecisionResponse, JevError> {
        let url = format!("{}/alpha/decisions", self.base_url);

        // NFR-3: the URL, model and question count are safe to log; the API
        // key and request body (which carries user content) never are.
        log::debug!(
            "Jev request: url={url} model={} questions={}",
            req.model,
            req.questions.len()
        );

        let send_result = self
            .client
            .post(&url)
            .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", self.api_key))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .timeout(self.timeout)
            .json(req)
            .send()
            .await;

        let response = match send_result {
            Ok(response) => response,
            Err(err) => {
                if err.is_timeout() {
                    return Err(JevError::Timeout);
                }
                return Err(JevError::Network(err.to_string()));
            }
        };

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(JevError::Http {
                status: status.as_u16(),
                body,
            });
        }

        let body_text = response.text().await.map_err(|err| JevError::Decode(err.to_string()))?;
        let decision_response: DecisionResponse =
            serde_json::from_str(&body_text).map_err(|err| JevError::Decode(err.to_string()))?;

        verify_answer_keys(req, &decision_response)?;

        Ok(decision_response)
    }
}

/// Every question asked must be answered, and only questions asked may be
/// answered — see the "Jev returns an answer key we didn't ask for, or leaves
/// one out" edge case.
fn verify_answer_keys(req: &DecisionRequest, resp: &DecisionResponse) -> Result<(), JevError> {
    let missing: Vec<String> = req
        .questions
        .keys()
        .filter(|k| !resp.answers.contains_key(*k))
        .cloned()
        .collect();
    let unexpected: Vec<String> = resp
        .answers
        .keys()
        .filter(|k| !req.questions.contains_key(*k))
        .cloned()
        .collect();

    if missing.is_empty() && unexpected.is_empty() {
        Ok(())
    } else {
        Err(JevError::AnswerKeysMismatch { missing, unexpected })
    }
}

impl fmt::Debug for HttpJev {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpJev")
            .field("api_key", &"[REDACTED]")
            .field("base_url", &self.base_url)
            .field("timeout", &self.timeout)
            .field("max_retries", &self.max_retries)
            .field("backoff_base", &self.backoff_base)
            .finish()
    }
}

#[async_trait]
impl DecisionClient for HttpJev {
    async fn decide(&self, req: &DecisionRequest) -> Result<DecisionResponse, JevError> {
        if self.api_key.is_empty() {
            return Err(JevError::NoApiKey);
        }

        let max_attempts = self.max_retries + 1;
        let mut last_err: Option<JevError> = None;

        for attempt in 0..max_attempts {
            if attempt > 0 {
                let backoff = self.backoff_base * 2u32.pow(attempt - 1);
                tokio::time::sleep(backoff).await;
            }

            match self.try_once(req).await {
                Ok(resp) => return Ok(resp),
                Err(err) => {
                    let retryable = err.is_retryable();
                    last_err = Some(err);
                    if !retryable {
                        return Err(last_err.expect("just set"));
                    }
                }
            }
        }

        Err(JevError::RetriesExhausted {
            attempts: max_attempts,
            last: Box::new(last_err.expect("loop ran at least once")),
        })
    }
}

/// A `DecisionClient` test double for other modules' tests: answers requests
/// through a closure, and records every request it was asked to decide.
pub struct FakeJev<F>
where
    F: Fn(&DecisionRequest) -> Result<DecisionResponse, JevError> + Send + Sync,
{
    handler: F,
    requests: Mutex<Vec<DecisionRequest>>,
}

impl<F> FakeJev<F>
where
    F: Fn(&DecisionRequest) -> Result<DecisionResponse, JevError> + Send + Sync,
{
    pub fn new(handler: F) -> Self {
        Self {
            handler,
            requests: Mutex::new(Vec::new()),
        }
    }

    /// Every request `decide` was called with, in call order.
    pub fn requests(&self) -> Vec<DecisionRequest> {
        self.requests.lock().expect("requests mutex poisoned").clone()
    }

    pub fn call_count(&self) -> usize {
        self.requests.lock().expect("requests mutex poisoned").len()
    }
}

#[async_trait]
impl<F> DecisionClient for FakeJev<F>
where
    F: Fn(&DecisionRequest) -> Result<DecisionResponse, JevError> + Send + Sync,
{
    async fn decide(&self, req: &DecisionRequest) -> Result<DecisionResponse, JevError> {
        self.requests
            .lock()
            .expect("requests mutex poisoned")
            .push(req.clone());
        (self.handler)(req)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_redacts_api_key() {
        let jev = HttpJev::new("sk-super-secret-key-123".to_string());
        let debug_str = format!("{:?}", jev);
        assert!(!debug_str.contains("sk-super-secret-key-123"));
        assert!(debug_str.contains("REDACTED"));
    }

    #[test]
    fn question_serializes_with_type_tag() {
        let q = Question::noul(Value::String("Is this a bug?".into()), "yes", "no");
        let json = serde_json::to_value(&q).unwrap();
        assert_eq!(json["type"], "noul");
        assert_eq!(json["criteria"]["true"], "yes");
        assert_eq!(json["criteria"]["false"], "no");
    }
}
