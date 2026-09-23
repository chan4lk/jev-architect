//! Variant-row selection (spec FR-9, design "Key Decisions" #4): pure,
//! data-driven rules that SELECT or EXCLUDE catalogue decision types chosen
//! by `rules.yaml`'s groups, once the platform stage has decided
//! `cloud-platform`, `backend-platform`, and `auth_app_type`. Rules never
//! pick a technology — only Jev's Choice questions (`questions.rs`) do that.

use crate::model::brief::{Brief, Budget};
use crate::model::catalog::{Catalog, DecisionType};

/// The `by` value `rules.yaml`'s `relational-db` group is keyed on. It isn't
/// a Jev-decided platform value like the others — it's derived here from the
/// brief's budget and the decided cloud platform (spec FR-9).
const RELATIONAL_DB_TIER_BY: &str = "relational-db-tier";

/// The three platform-stage values Jev decides (design "Jev request
/// shapes"), each a catalogue option id: `cloud-platform`'s, `backend-platform`'s,
/// and `rules.auth_app_types`' respectively.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformOutcome {
    pub cloud: String,
    pub backend: String,
    pub auth_app_type: String,
}

/// Returns the decision-type ids that should proceed to applicability and
/// decision (spec FR-9, AC-8): every catalogue type with a `matrix` row,
/// filtered by `rules.yaml`'s groups where one applies.
///
/// A type whose `group` has no matching `rules.yaml` group, or that has no
/// `group` at all, is always kept. A type whose group *is* covered is kept
/// only when its id is listed under the decided value; an unrecognised
/// decided value (one `rules.yaml` doesn't list) keeps every type in that
/// group, as a safe default. `cloud-platform` and `backend-platform`
/// themselves are never returned, since rules only run after they're
/// decided. Order matches the catalogue's own type order.
pub fn candidate_types(cat: &Catalog, outcome: &PlatformOutcome, brief: &Brief) -> Vec<String> {
    let relational_db_tier = if brief.context.budget == Budget::Tight || outcome.cloud == "hetzner"
    {
        "budget"
    } else {
        "enterprise"
    };

    cat.types
        .iter()
        .filter(|t| t.matrix.is_some())
        .filter(|t| is_candidate(cat, t, outcome, relational_db_tier))
        .map(|t| t.id.clone())
        .collect()
}

fn is_candidate(
    cat: &Catalog,
    t: &DecisionType,
    outcome: &PlatformOutcome,
    relational_db_tier: &str,
) -> bool {
    let Some(group) = &t.group else {
        return true;
    };
    let Some(rule) = cat.rules.groups.iter().find(|g| &g.group == group) else {
        return true;
    };

    let decided_value = match rule.by.as_str() {
        "backend-platform" => outcome.backend.as_str(),
        "cloud-platform" => outcome.cloud.as_str(),
        "auth_app_type" => outcome.auth_app_type.as_str(),
        RELATIONAL_DB_TIER_BY => relational_db_tier,
        // An unrecognised `by` value: keep the type rather than silently drop it.
        _ => return true,
    };

    match rule.select.get(decided_value) {
        Some(ids) => ids.iter().any(|id| id == &t.id),
        // Unknown decided value (e.g. a not-yet-modelled outcome): keep every
        // type in the group rather than guess which ones apply.
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::brief::{Brief, ContextAssessment};
    use std::collections::BTreeSet;

    fn catalog() -> Catalog {
        Catalog::bundled().expect("bundled catalogue should load and validate")
    }

    fn brief_with_budget(budget: Budget) -> Brief {
        Brief {
            summary: "test".to_string(),
            context: ContextAssessment {
                budget,
                ..ContextAssessment::default()
            },
            requirements: vec![],
            nfrs: vec![],
            constraints: vec![],
            team_skills: vec![],
            mentioned_technologies: vec![],
        }
    }

    fn outcome(cloud: &str, backend: &str, auth_app_type: &str) -> PlatformOutcome {
        PlatformOutcome {
            cloud: cloud.to_string(),
            backend: backend.to_string(),
            auth_app_type: auth_app_type.to_string(),
        }
    }

    #[test]
    fn dotnet_backend_selects_only_the_dotnet_rest_api_and_testing_rows() {
        let cat = catalog();
        let types = candidate_types(
            &cat,
            &outcome("azure", "dotnet", "internal_enterprise"),
            &brief_with_budget(Budget::Enterprise),
        );
        let set: BTreeSet<&str> = types.iter().map(|s| s.as_str()).collect();

        assert!(set.contains("rest-api-dotnet"));
        assert!(set.contains("testing-dotnet"));
        assert!(!set.contains("rest-api-node"));
        assert!(!set.contains("testing-node"));
    }

    #[test]
    fn tight_budget_selects_the_relational_db_budget_row() {
        let cat = catalog();
        let types = candidate_types(
            &cat,
            &outcome("azure", "dotnet", "internal_enterprise"),
            &brief_with_budget(Budget::Tight),
        );
        let set: BTreeSet<&str> = types.iter().map(|s| s.as_str()).collect();
        assert!(set.contains("relational-db-budget"));
        assert!(!set.contains("relational-db-enterprise"));
    }

    #[test]
    fn hetzner_cloud_selects_the_relational_db_budget_row_even_with_enterprise_budget() {
        let cat = catalog();
        let types = candidate_types(
            &cat,
            &outcome("hetzner", "dotnet", "internal_enterprise"),
            &brief_with_budget(Budget::Enterprise),
        );
        let set: BTreeSet<&str> = types.iter().map(|s| s.as_str()).collect();
        assert!(set.contains("relational-db-budget"));
        assert!(!set.contains("relational-db-enterprise"));
    }

    #[test]
    fn enterprise_budget_on_azure_selects_the_relational_db_enterprise_row() {
        let cat = catalog();
        let types = candidate_types(
            &cat,
            &outcome("azure", "dotnet", "internal_enterprise"),
            &brief_with_budget(Budget::Enterprise),
        );
        let set: BTreeSet<&str> = types.iter().map(|s| s.as_str()).collect();
        assert!(set.contains("relational-db-enterprise"));
        assert!(!set.contains("relational-db-budget"));
    }

    #[test]
    fn hybrid_cloud_selects_both_iac_rows() {
        let cat = catalog();
        let types = candidate_types(
            &cat,
            &outcome("hybrid", "dotnet", "internal_enterprise"),
            &brief_with_budget(Budget::Enterprise),
        );
        let set: BTreeSet<&str> = types.iter().map(|s| s.as_str()).collect();
        assert!(set.contains("iac-azure"));
        assert!(set.contains("iac-hetzner"));
    }

    #[test]
    fn an_unrecognised_backend_value_keeps_every_type_in_the_group() {
        let cat = catalog();
        let types = candidate_types(
            &cat,
            &outcome("azure", "cobol", "internal_enterprise"),
            &brief_with_budget(Budget::Enterprise),
        );
        let set: BTreeSet<&str> = types.iter().map(|s| s.as_str()).collect();
        assert!(set.contains("rest-api-dotnet"));
        assert!(set.contains("rest-api-node"));
        assert!(set.contains("testing-dotnet"));
        assert!(set.contains("testing-node"));
    }

    #[test]
    fn no_rule_ever_outputs_a_platform_type_or_a_technology_option_id() {
        let cat = catalog();
        let all_type_ids: BTreeSet<&str> = cat.types.iter().map(|t| t.id.as_str()).collect();
        // Option ids that never also name a decision type: if a returned id
        // were ever a technology instead of a type, it would show up here.
        let option_only_ids: BTreeSet<&str> = cat
            .types
            .iter()
            .flat_map(|t| t.options.iter().map(|o| o.id.as_str()))
            .filter(|id| !all_type_ids.contains(id))
            .collect();
        assert!(
            !option_only_ids.is_empty(),
            "sanity check: the catalogue has option ids to test against"
        );

        let types = candidate_types(
            &cat,
            &outcome("hybrid", "java-python", "b2c"),
            &brief_with_budget(Budget::Tight),
        );

        assert!(!types.is_empty());
        for id in &types {
            assert!(
                all_type_ids.contains(id.as_str()),
                "{id} is not a catalogue type id"
            );
            assert!(
                !option_only_ids.contains(id.as_str()),
                "{id} looks like a technology option id, not a type id"
            );
            assert_ne!(id, "cloud-platform");
            assert_ne!(id, "backend-platform");
        }
    }

    #[test]
    fn matrix_type_with_no_group_is_always_kept() {
        let cat = catalog();
        let dt = cat
            .type_by_id("frontend")
            .expect("frontend type should exist");
        assert!(
            dt.group.is_none(),
            "this test assumes 'frontend' has no rule group"
        );

        let types = candidate_types(
            &cat,
            &outcome("azure", "dotnet", "internal_enterprise"),
            &brief_with_budget(Budget::Enterprise),
        );
        assert!(types.contains(&"frontend".to_string()));
    }

    #[test]
    fn a_group_with_no_matching_rule_is_always_kept() {
        let cat = catalog();
        // message-queue-* types have a `group`, but rules.yaml has no
        // "message-queue" rule group — applicability decides them instead.
        let types = candidate_types(
            &cat,
            &outcome("azure", "dotnet", "internal_enterprise"),
            &brief_with_budget(Budget::Enterprise),
        );
        assert!(types.contains(&"message-queue-enterprise".to_string()));
        assert!(types.contains(&"message-queue-simple".to_string()));
    }
}
