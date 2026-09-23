//! Serde DTO for the app's user-configurable settings (spec FR-1).
//!
//! `Settings` never carries the OpenRouter API key (spec FR-2, NFR-3): the
//! key lives only in the OS credential store, via `crate::secrets`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The first-run data-notice text (spec FR-3): shown before the first Jev
/// call, and again in Settings. Acknowledgement is persisted by
/// `Store::ack_data_notice`.
pub const DATA_NOTICE: &str = "The content of your project brief and any uploaded documents is sent, \
unredacted, to OpenRouter and TypeSafe to produce Jev's technology decisions. \
Nothing else leaves this machine: the local model (MiniCPM, via Ollama) runs \
entirely on this computer.";

/// User-configurable settings (spec FR-1). `#[serde(default)]` on the
/// container means older stored JSON missing newer fields still loads,
/// falling back to `Settings::default()` for whatever is missing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
  pub jev_model: String,
  pub jev_base_url: String,
  pub ollama_base_url: String,
  pub ollama_model: String,
  pub state_token_budget: usize,
  pub confidence_threshold: f64,
  pub min_margin: f64,
  pub max_questions_per_call: usize,
  pub max_concurrent_calls: usize,
  /// Weight per criterion id. Empty means "use the catalogue's defaults".
  pub weights: BTreeMap<String, f64>,
  pub reviewer_name: String,
}

impl Default for Settings {
  fn default() -> Self {
    Settings {
      jev_model: "typesafe/jev-1.13".to_string(),
      jev_base_url: "https://openrouter.ai/api".to_string(),
      ollama_base_url: "http://127.0.0.1:11434".to_string(),
      ollama_model: "hf.co/openbmb/MiniCPM5-2B-GGUF:Q4_K_M".to_string(),
      state_token_budget: 20_000,
      confidence_threshold: 0.5,
      min_margin: 0.05,
      max_questions_per_call: 64,
      max_concurrent_calls: 4,
      weights: BTreeMap::new(),
      reviewer_name: String::new(),
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn defaults_match_spec_a1() {
    let s = Settings::default();
    assert_eq!(s.jev_model, "typesafe/jev-1.13");
    assert_eq!(s.jev_base_url, "https://openrouter.ai/api");
    assert_eq!(s.ollama_base_url, "http://127.0.0.1:11434");
    assert_eq!(s.ollama_model, "hf.co/openbmb/MiniCPM5-2B-GGUF:Q4_K_M");
    assert_eq!(s.state_token_budget, 20_000);
    assert_eq!(s.confidence_threshold, 0.5);
    assert_eq!(s.min_margin, 0.05);
    assert_eq!(s.max_questions_per_call, 64);
    assert_eq!(s.max_concurrent_calls, 4);
    assert!(s.weights.is_empty());
    assert_eq!(s.reviewer_name, "");
  }

  #[test]
  fn older_json_missing_newer_fields_still_loads() {
    let old_json = r#"{ "jev_model": "typesafe/jev-1.13" }"#;
    let s: Settings = serde_json::from_str(old_json).expect("partial JSON should deserialize");
    assert_eq!(s.jev_model, "typesafe/jev-1.13");
    // Everything else falls back to Settings::default().
    assert_eq!(s.reviewer_name, "");
    assert_eq!(s.state_token_budget, 20_000);
  }

  #[test]
  fn serializes_without_an_api_key_field() {
    let json = serde_json::to_value(Settings::default()).unwrap();
    assert!(json.get("api_key").is_none());
    assert!(json.get("openrouter_api_key").is_none());
  }
}
