//! The Brief DTO and its Context Assessment enums.
//!
//! Mirrors the `bistec-architect` skill's Context Assessment: a handful of
//! discrete project-context dimensions (scale, budget, timeline, team size,
//! compliance, data sensitivity) that MiniCPM extracts from free text or
//! documents, and that Jev later asks about directly when the evidence
//! doesn't state them (Mode B small path).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Expected number of users / load the project must support.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum Scale {
    /// Small: fewer than 1,000 users.
    Small,
    /// Medium: fewer than 100,000 users.
    Medium,
    /// Large: 100,000+ users.
    Large,
    /// Not stated in the evidence.
    #[default]
    Unknown,
}

impl Scale {
    pub fn describe(&self) -> &'static str {
        match self {
            Scale::Small => "Small: fewer than 1,000 users.",
            Scale::Medium => "Medium: fewer than 100,000 users.",
            Scale::Large => "Large: 100,000+ users.",
            Scale::Unknown => "Not stated in the evidence.",
        }
    }
}

/// Monthly budget available for infrastructure and services.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum Budget {
    /// Tight: less than $500/mo.
    Tight,
    /// Moderate: less than $5,000/mo.
    Moderate,
    /// Enterprise: more than $5,000/mo.
    Enterprise,
    /// Not stated in the evidence.
    #[default]
    Unknown,
}

impl Budget {
    pub fn describe(&self) -> &'static str {
        match self {
            Budget::Tight => "Tight: less than $500/mo.",
            Budget::Moderate => "Moderate: less than $5,000/mo.",
            Budget::Enterprise => "Enterprise: more than $5,000/mo.",
            Budget::Unknown => "Not stated in the evidence.",
        }
    }
}

/// How soon the project must ship.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum Timeline {
    /// Urgent: less than 4 weeks.
    Urgent,
    /// Normal: 1-3 months.
    Normal,
    /// LongTerm: 3+ months.
    LongTerm,
    /// Not stated in the evidence.
    #[default]
    Unknown,
}

impl Timeline {
    pub fn describe(&self) -> &'static str {
        match self {
            Timeline::Urgent => "Urgent: less than 4 weeks.",
            Timeline::Normal => "Normal: 1-3 months.",
            Timeline::LongTerm => "LongTerm: 3+ months.",
            Timeline::Unknown => "Not stated in the evidence.",
        }
    }
}

/// Size of the team building and operating the project.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum TeamSize {
    /// SoloPair: a solo developer or a pair.
    SoloPair,
    /// Small: 3-5 people.
    Small,
    /// Large: 5+ people.
    Large,
    /// Not stated in the evidence.
    #[default]
    Unknown,
}

impl TeamSize {
    pub fn describe(&self) -> &'static str {
        match self {
            TeamSize::SoloPair => "SoloPair: a solo developer or a pair.",
            TeamSize::Small => "Small: 3-5 people.",
            TeamSize::Large => "Large: 5+ people.",
            TeamSize::Unknown => "Not stated in the evidence.",
        }
    }
}

/// A compliance regime the project must satisfy. Listed only when stated;
/// an empty `Vec<Compliance>` means none was stated.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Compliance {
    Soc2,
    Gdpr,
    Hipaa,
    IndustrySpecific,
}

impl Compliance {
    pub fn describe(&self) -> &'static str {
        match self {
            Compliance::Soc2 => "SOC 2.",
            Compliance::Gdpr => "GDPR.",
            Compliance::Hipaa => "HIPAA.",
            Compliance::IndustrySpecific => "An industry-specific regulatory regime.",
        }
    }
}

/// Sensitivity of the data the project handles.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum DataSensitivity {
    /// Public: no confidentiality requirements.
    Public,
    /// Internal: for internal use only.
    Internal,
    /// Confidential: sensitive business or personal data.
    Confidential,
    /// Restricted: highly sensitive, regulated, or classified data.
    Restricted,
    /// Not stated in the evidence.
    #[default]
    Unknown,
}

impl DataSensitivity {
    pub fn describe(&self) -> &'static str {
        match self {
            DataSensitivity::Public => "Public: no confidentiality requirements.",
            DataSensitivity::Internal => "Internal: for internal use only.",
            DataSensitivity::Confidential => "Confidential: sensitive business or personal data.",
            DataSensitivity::Restricted => {
                "Restricted: highly sensitive, regulated, or classified data."
            }
            DataSensitivity::Unknown => "Not stated in the evidence.",
        }
    }
}

/// The project-context dimensions from the `bistec-architect` Context
/// Assessment. Any dimension not explicitly stated in the evidence is
/// `Unknown` (or, for `compliance`, an empty list) — it is never guessed.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub struct ContextAssessment {
    pub scale: Scale,
    pub budget: Budget,
    pub timeline: Timeline,
    pub team_size: TeamSize,
    #[serde(default)]
    pub compliance: Vec<Compliance>,
    pub data_sensitivity: DataSensitivity,
}

/// One atomic item (a requirement, NFR, constraint, or team-skill note).
///
/// `sources` holds the section ids (e.g. "S3") the item was extracted from.
/// Mode A never has evidence sections, so it is always empty there.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct BriefItem {
    pub text: String,
    #[serde(default)]
    pub sources: Vec<String>,
}

/// The structured project brief extracted from free text or a document.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Brief {
    pub summary: String,
    pub context: ContextAssessment,
    #[serde(default)]
    pub requirements: Vec<BriefItem>,
    #[serde(default)]
    pub nfrs: Vec<BriefItem>,
    #[serde(default)]
    pub constraints: Vec<BriefItem>,
    #[serde(default)]
    pub team_skills: Vec<BriefItem>,
    #[serde(default)]
    pub mentioned_technologies: Vec<String>,
}

/// A JSON Schema for [`Brief`], suitable for Ollama's `format` field.
///
/// Schemars' default output can nest definitions behind `$ref`; Ollama's
/// structured-output support (and small local models) handle an inlined,
/// `$ref`-free schema more reliably, so references are expanded in place.
pub fn brief_json_schema() -> serde_json::Value {
    let settings = schemars::generate::SchemaSettings::default().with(|s| {
        s.inline_subschemas = true;
    });
    let generator = schemars::SchemaGenerator::new(settings);
    let schema = generator.into_root_schema_for::<Brief>();
    serde_json::Value::from(schema)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_context_assessment_is_all_unknown() {
        let ctx = ContextAssessment::default();
        assert_eq!(ctx.scale, Scale::Unknown);
        assert_eq!(ctx.budget, Budget::Unknown);
        assert_eq!(ctx.timeline, Timeline::Unknown);
        assert_eq!(ctx.team_size, TeamSize::Unknown);
        assert!(ctx.compliance.is_empty());
        assert_eq!(ctx.data_sensitivity, DataSensitivity::Unknown);
    }

    #[test]
    fn brief_round_trips_through_json() {
        let brief = Brief {
            summary: "A small internal tool".to_string(),
            context: ContextAssessment {
                scale: Scale::Small,
                budget: Budget::Tight,
                timeline: Timeline::Urgent,
                team_size: TeamSize::SoloPair,
                compliance: vec![Compliance::Gdpr],
                data_sensitivity: DataSensitivity::Internal,
            },
            requirements: vec![BriefItem {
                text: "Must support CSV export".to_string(),
                sources: vec![],
            }],
            nfrs: vec![],
            constraints: vec![],
            team_skills: vec![],
            mentioned_technologies: vec!["PostgreSQL".to_string()],
        };
        let json = serde_json::to_string(&brief).unwrap();
        let round_tripped: Brief = serde_json::from_str(&json).unwrap();
        assert_eq!(brief, round_tripped);
    }

    #[test]
    fn schema_is_object_containing_context_property() {
        let schema = brief_json_schema();
        let obj = schema.as_object().expect("schema is a JSON object");
        let properties = obj
            .get("properties")
            .and_then(|p| p.as_object())
            .expect("schema has a properties object");
        assert!(properties.contains_key("context"));
    }

    #[test]
    fn schema_has_no_refs() {
        let schema = brief_json_schema();
        let text = serde_json::to_string(&schema).unwrap();
        assert!(
            !text.contains("$ref"),
            "schema should be fully inlined for Ollama's format field, got: {text}"
        );
    }
}
