//! The `LocalModel` trait and its Ollama implementation.
//!
//! MiniCPM (served locally by Ollama) *understands*: it extracts a
//! structured Brief from free text or document sections, through Ollama's
//! `/api/chat` endpoint with a JSON-schema `format`. It never makes
//! decisions — that's Jev's job (see `jev.rs`).

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;
use thiserror::Error;

/// The default local model, served by Ollama (A5).
pub const DEFAULT_MODEL: &str = "hf.co/openbmb/MiniCPM5-2B-GGUF:Q4_K_M";
/// The default Ollama base URL (A5).
pub const DEFAULT_BASE_URL: &str = "http://127.0.0.1:11434";

/// Errors from talking to a local model server.
#[derive(Debug, Error)]
pub enum LocalModelError {
    #[error("could not reach the local model server: {0}")]
    Unreachable(String),
    #[error("local model server returned HTTP {status}: {body}")]
    Http { status: u16, body: String },
    #[error("local model server request timed out")]
    Timeout,
    #[error("could not decode the local model server's response: {0}")]
    Decode(String),
}

/// Understands: extracts structured JSON from free text against a schema.
#[async_trait]
pub trait LocalModel: Send + Sync {
    /// Sends a system/user chat turn constrained to `schema` and returns the
    /// raw `message.content` string from the model's response. Parsing and
    /// validating that string as JSON is the caller's job.
    async fn chat_json(
        &self,
        system: &str,
        user: &str,
        schema: &Value,
    ) -> Result<String, LocalModelError>;

    /// Whether the configured model is pulled and ready.
    async fn model_present(&self) -> Result<bool, LocalModelError>;

    /// The configured model name.
    fn model_name(&self) -> &str;
}

/// The exact shell command to pull the given model (AC-20).
pub fn pull_command(model: &str) -> String {
    format!("ollama pull {model}")
}

/// Adds an implicit `:latest` tag so `"llama3"` and `"llama3:latest"`
/// compare equal, without disturbing a name that already carries a tag
/// (such as `"hf.co/openbmb/MiniCPM5-2B-GGUF:Q4_K_M"`).
fn normalize_tag(name: &str) -> String {
    if name.contains(':') {
        name.to_string()
    } else {
        format!("{name}:latest")
    }
}

/// `LocalModel` implemented over Ollama's HTTP API.
pub struct OllamaModel {
    base_url: String,
    model: String,
    client: reqwest::Client,
}

impl OllamaModel {
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .expect("reqwest client with a fixed timeout always builds");
        Self {
            base_url: base_url.into(),
            model: model.into(),
            client,
        }
    }

}

#[async_trait]
impl LocalModel for OllamaModel {
    async fn chat_json(
        &self,
        system: &str,
        user: &str,
        schema: &Value,
    ) -> Result<String, LocalModelError> {
        let url = format!("{}/api/chat", self.base_url);
        let body = serde_json::json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user},
            ],
            "format": schema,
            "stream": false,
            "options": {"temperature": 0},
        });

        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    LocalModelError::Timeout
                } else {
                    LocalModelError::Unreachable(e.to_string())
                }
            })?;

        let status = resp.status();
        if !status.is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            return Err(LocalModelError::Http {
                status: status.as_u16(),
                body: body_text,
            });
        }

        let value: Value = resp
            .json()
            .await
            .map_err(|e| LocalModelError::Decode(e.to_string()))?;

        value
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| LocalModelError::Decode("response has no message.content".to_string()))
    }

    async fn model_present(&self) -> Result<bool, LocalModelError> {
        let url = format!("{}/api/tags", self.base_url);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    LocalModelError::Timeout
                } else {
                    LocalModelError::Unreachable(e.to_string())
                }
            })?;

        let status = resp.status();
        if !status.is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            return Err(LocalModelError::Http {
                status: status.as_u16(),
                body: body_text,
            });
        }

        let value: Value = resp
            .json()
            .await
            .map_err(|e| LocalModelError::Decode(e.to_string()))?;

        let configured = normalize_tag(&self.model);
        let present = value
            .get("models")
            .and_then(|m| m.as_array())
            .map(|models| {
                models.iter().any(|m| {
                    let name = m
                        .get("name")
                        .and_then(|n| n.as_str())
                        .or_else(|| m.get("model").and_then(|n| n.as_str()));
                    name.map(|n| normalize_tag(n) == configured).unwrap_or(false)
                })
            })
            .unwrap_or(false);

        Ok(present)
    }

    fn model_name(&self) -> &str {
        &self.model
    }
}

/// A fake `LocalModel` for offline tests: replays a fixed queue of results
/// and records every call it received.
pub struct FakeLocalModel {
    model: String,
    present: bool,
    /// When set, `model_present` fails as if the Ollama server couldn't be
    /// reached at all, instead of reporting `present`.
    unreachable: bool,
    responses: Mutex<VecDeque<Result<String, LocalModelError>>>,
    calls: Mutex<Vec<(String, String)>>,
}

impl FakeLocalModel {
    pub fn new(
        model: impl Into<String>,
        present: bool,
        responses: Vec<Result<String, LocalModelError>>,
    ) -> Self {
        Self {
            model: model.into(),
            present,
            unreachable: false,
            responses: Mutex::new(VecDeque::from(responses)),
            calls: Mutex::new(Vec::new()),
        }
    }

    /// A fake whose server is down: `model_present` fails with
    /// `LocalModelError::Unreachable` (simulates "Ollama not running").
    pub fn unreachable(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            present: false,
            unreachable: true,
            responses: Mutex::new(VecDeque::new()),
            calls: Mutex::new(Vec::new()),
        }
    }

    /// The `(system, user)` pairs passed to every `chat_json` call so far, in order.
    pub fn calls(&self) -> Vec<(String, String)> {
        self.calls.lock().expect("lock poisoned").clone()
    }
}

#[async_trait]
impl LocalModel for FakeLocalModel {
    async fn chat_json(
        &self,
        system: &str,
        user: &str,
        _schema: &Value,
    ) -> Result<String, LocalModelError> {
        self.calls
            .lock()
            .expect("lock poisoned")
            .push((system.to_string(), user.to_string()));
        self.responses
            .lock()
            .expect("lock poisoned")
            .pop_front()
            .unwrap_or_else(|| {
                Err(LocalModelError::Decode(
                    "FakeLocalModel: no more canned responses".to_string(),
                ))
            })
    }

    async fn model_present(&self) -> Result<bool, LocalModelError> {
        if self.unreachable {
            return Err(LocalModelError::Unreachable("fake: server unreachable".to_string()));
        }
        Ok(self.present)
    }

    fn model_name(&self) -> &str {
        &self.model
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pull_command_for_default_model() {
        assert_eq!(
            pull_command(DEFAULT_MODEL),
            "ollama pull hf.co/openbmb/MiniCPM5-2B-GGUF:Q4_K_M"
        );
    }

    #[test]
    fn normalize_tag_adds_latest_when_untagged() {
        assert_eq!(normalize_tag("llama3"), "llama3:latest");
        assert_eq!(normalize_tag("llama3:latest"), "llama3:latest");
        assert_eq!(
            normalize_tag("hf.co/openbmb/MiniCPM5-2B-GGUF:Q4_K_M"),
            "hf.co/openbmb/MiniCPM5-2B-GGUF:Q4_K_M"
        );
    }

    #[tokio::test]
    async fn fake_local_model_records_calls_and_replays_responses() {
        let fake = FakeLocalModel::new(
            "test-model",
            true,
            vec![Ok("{}".to_string()), Err(LocalModelError::Decode("bad".to_string()))],
        );
        let schema = serde_json::json!({});
        let first = fake.chat_json("sys", "user1", &schema).await;
        assert_eq!(first.unwrap(), "{}");
        let second = fake.chat_json("sys", "user2", &schema).await;
        assert!(second.is_err());
        assert_eq!(
            fake.calls(),
            vec![
                ("sys".to_string(), "user1".to_string()),
                ("sys".to_string(), "user2".to_string())
            ]
        );
        assert!(fake.model_present().await.unwrap());
        assert_eq!(fake.model_name(), "test-model");
    }
}
