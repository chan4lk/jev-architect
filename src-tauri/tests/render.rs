//! Integration tests for `bistec_architect::render` (spec FR-19, FR-20;
//! AC-14, AC-15).
//!
//! Golden files live in `tests/fixtures/adr/`. Regenerate them with:
//!
//! ```sh
//! UPDATE_GOLDEN=1 cargo test --manifest-path src-tauri/Cargo.toml --test render
//! ```
//!
//! then re-run without the env var (and commit the fixtures) to confirm they
//! match on a normal run.

use std::collections::BTreeMap;
use std::path::PathBuf;

use chrono::{NaiveDate, TimeZone, Utc};

use bistec_architect::catalog::Catalog;
use bistec_architect::model::brief::{
    Brief, BriefItem, Budget, Compliance, ContextAssessment, DataSensitivity, Scale, TeamSize,
    Timeline,
};
use bistec_architect::model::decision::{DecisionResult, OptionScore, ReasonCode, Route};
use bistec_architect::model::report::{
    CriterionView, DecisionView, Gates, NotApplicableView, OptionView, Report,
};
use bistec_architect::model::review::{adr_status, Review, ReviewAction};
use bistec_architect::render::{
    export_adrs, next_adr_number, render_adr, render_report_html, render_report_md, slugify,
    RenderContext,
};

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/adr")
}

/// Compares `actual` against `tests/fixtures/adr/<name>`. With `UPDATE_GOLDEN=1`
/// set, writes `actual` as the new fixture instead of comparing.
fn assert_matches_golden(name: &str, actual: &str) {
    let path = fixture_dir().join(name);

    if std::env::var("UPDATE_GOLDEN").is_ok() {
        std::fs::write(&path, actual)
            .unwrap_or_else(|e| panic!("failed to write golden {path:?}: {e}"));
        return;
    }

    let expected = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!("failed to read golden {path:?}: {e} (run with UPDATE_GOLDEN=1 to create it)")
    });
    assert_eq!(
    actual, expected,
    "rendered output does not match golden file {path:?} (run with UPDATE_GOLDEN=1 to update it if the change is intentional)"
  );
}

fn options_for(cat: &Catalog, type_id: &str) -> Vec<OptionView> {
    cat.type_by_id(type_id)
        .unwrap_or_else(|| panic!("catalog has no decision type {type_id:?}"))
        .options
        .iter()
        .map(|o| OptionView {
            id: o.id.clone(),
            name: o.name.clone(),
            ring: o.ring,
            description: o.description.clone(),
        })
        .collect()
}

fn scores(pairs: &[(&str, f64)]) -> BTreeMap<String, f64> {
    pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
}

fn probs(pairs: &[(&str, f64)]) -> BTreeMap<String, f64> {
    pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
}

fn item(text: &str, sources: &[&str]) -> BriefItem {
    BriefItem {
        text: text.to_string(),
        sources: sources.iter().map(|s| s.to_string()).collect(),
    }
}

fn fixture_brief() -> Brief {
    Brief {
        summary: "A small internal claims-processing tool for policyholder self-service."
            .to_string(),
        context: ContextAssessment {
            scale: Scale::Small,
            budget: Budget::Tight,
            timeline: Timeline::Urgent,
            team_size: TeamSize::SoloPair,
            compliance: vec![Compliance::Gdpr],
            data_sensitivity: DataSensitivity::Confidential,
        },
        requirements: vec![item(
            "Support policyholder login via Microsoft 365 SSO",
            &["S1"],
        )],
        nfrs: vec![item("Must handle 500 concurrent users at peak", &["S2"])],
        constraints: vec![item("Budget capped at $400/month", &["S1"])],
        team_skills: vec![item("Team knows <b>C#</b> and Azure", &[])],
        mentioned_technologies: vec!["Azure".to_string(), "PostgreSQL".to_string()],
    }
}

/// Decision 1: `cloud-platform`, adopt choice, Accepted.
fn cloud_platform_decision(cat: &Catalog) -> DecisionView {
    let decision = DecisionResult {
        id: "d-cloud-platform".to_string(),
        session_id: "s1".to_string(),
        type_id: "cloud-platform".to_string(),
        choice: "azure".to_string(),
        confidence: 0.92,
        probabilities: probs(&[("azure", 0.85), ("hetzner", 0.10), ("hybrid", 0.05)]),
        option_scores: vec![
            OptionScore {
                option_id: "azure".to_string(),
                criterion_scores: scores(&[
                    ("nfr_fit", 4.0),
                    ("team_skill_fit", 3.0),
                    ("cost_fit", 3.0),
                    ("bistec_alignment", 4.0),
                    ("lock_in", 2.0),
                    ("security_posture", 4.0),
                ]),
                composite: 0.860,
            },
            OptionScore {
                option_id: "hetzner".to_string(),
                criterion_scores: scores(&[
                    ("nfr_fit", 2.0),
                    ("team_skill_fit", 2.0),
                    ("cost_fit", 4.0),
                    ("bistec_alignment", 3.0),
                    ("lock_in", 3.0),
                    ("security_posture", 3.0),
                ]),
                composite: 0.625,
            },
            OptionScore {
                option_id: "hybrid".to_string(),
                criterion_scores: scores(&[
                    ("nfr_fit", 3.0),
                    ("team_skill_fit", 2.0),
                    ("cost_fit", 3.0),
                    ("bistec_alignment", 3.0),
                    ("lock_in", 2.0),
                    ("security_posture", 3.0),
                ]),
                composite: 0.675,
            },
        ],
        route: Route::Proposed,
        reasons: vec![],
        model_snapshot: "typesafe/jev-1.13-2026-01-01".to_string(),
        request_hash: "hash-cloud-platform".to_string(),
        cited_sections: vec!["S1".to_string(), "S2".to_string()],
    };

    let reviews = vec![Review {
        id: 1,
        decision_id: decision.id.clone(),
        action: ReviewAction::Accept,
        option_id: None,
        reviewer: "Alex Architect".to_string(),
        reason: None,
        at_utc: Utc.with_ymd_and_hms(2026, 9, 20, 10, 0, 0).unwrap(),
    }];
    let status = adr_status(&reviews);

    DecisionView {
        decision,
        type_name: "Cloud platform".to_string(),
        options: options_for(cat, "cloud-platform"),
        status_label: status.label().to_string(),
        status,
        reviews,
    }
}

/// Decision 2: `message-queue-simple`, trial choice, overridden.
fn message_queue_decision(cat: &Catalog) -> DecisionView {
    let decision = DecisionResult {
        id: "d-message-queue-simple".to_string(),
        session_id: "s1".to_string(),
        type_id: "message-queue-simple".to_string(),
        choice: "redis-pubsub".to_string(),
        confidence: 0.60,
        probabilities: probs(&[
            ("azure-queue-storage", 0.30),
            ("redis-pubsub", 0.55),
            ("kafka", 0.15),
        ]),
        option_scores: vec![
            OptionScore {
                option_id: "azure-queue-storage".to_string(),
                criterion_scores: scores(&[
                    ("nfr_fit", 3.0),
                    ("team_skill_fit", 3.0),
                    ("cost_fit", 4.0),
                    ("bistec_alignment", 4.0),
                    ("lock_in", 3.0),
                    ("security_posture", 3.0),
                ]),
                composite: 0.780,
            },
            OptionScore {
                option_id: "redis-pubsub".to_string(),
                criterion_scores: scores(&[
                    ("nfr_fit", 3.0),
                    ("team_skill_fit", 4.0),
                    ("cost_fit", 4.0),
                    ("bistec_alignment", 2.0),
                    ("lock_in", 3.0),
                    ("security_posture", 2.0),
                ]),
                composite: 0.700,
            },
            // kafka is on hold: FR-12 never asks Score questions for a hold option.
        ],
        route: Route::Proposed,
        reasons: vec![],
        model_snapshot: "typesafe/jev-1.13-2026-01-01".to_string(),
        request_hash: "hash-message-queue-simple".to_string(),
        cited_sections: vec![],
    };

    let reviews = vec![Review {
        id: 2,
        decision_id: decision.id.clone(),
        action: ReviewAction::Override,
        option_id: Some("azure-queue-storage".to_string()),
        reviewer: "Alex Architect".to_string(),
        reason: Some(
            "Team already runs Azure Queue Storage elsewhere; avoid new infra.".to_string(),
        ),
        at_utc: Utc.with_ymd_and_hms(2026, 9, 21, 9, 30, 0).unwrap(),
    }];
    let status = adr_status(&reviews);

    DecisionView {
        decision,
        type_name: "Message queue (simple)".to_string(),
        options: options_for(cat, "message-queue-simple"),
        status_label: status.label().to_string(),
        status,
        reviews,
    }
}

/// Decision 3: `relational-db-budget`, hold choice, unreviewed, Needs architect.
fn relational_db_decision(cat: &Catalog) -> DecisionView {
    let decision = DecisionResult {
        id: "d-relational-db-budget".to_string(),
        session_id: "s1".to_string(),
        type_id: "relational-db-budget".to_string(),
        choice: "mysql".to_string(),
        confidence: 0.40,
        probabilities: probs(&[
            ("postgresql-hetzner", 0.30),
            ("sqlite-tiny", 0.25),
            ("mysql", 0.45),
        ]),
        option_scores: vec![
            OptionScore {
                option_id: "postgresql-hetzner".to_string(),
                criterion_scores: scores(&[
                    ("nfr_fit", 3.0),
                    ("team_skill_fit", 2.0),
                    ("cost_fit", 4.0),
                    ("bistec_alignment", 4.0),
                    ("lock_in", 3.0),
                    ("security_posture", 3.0),
                ]),
                composite: 0.740,
            },
            OptionScore {
                option_id: "sqlite-tiny".to_string(),
                criterion_scores: scores(&[
                    ("nfr_fit", 1.0),
                    ("team_skill_fit", 2.0),
                    ("cost_fit", 4.0),
                    ("bistec_alignment", 3.0),
                    ("lock_in", 2.0),
                    ("security_posture", 1.0),
                ]),
                composite: 0.430,
            },
            // mysql is on hold: not scored.
        ],
        route: Route::NeedsArchitect,
        reasons: vec![ReasonCode::HoldOption, ReasonCode::LowConfidence],
        model_snapshot: "typesafe/jev-1.13-2026-01-01".to_string(),
        request_hash: "hash-relational-db-budget".to_string(),
        cited_sections: vec!["S3".to_string()],
    };

    let reviews: Vec<Review> = vec![];
    let status = adr_status(&reviews);

    DecisionView {
        decision,
        type_name: "Relational DB (budget)".to_string(),
        options: options_for(cat, "relational-db-budget"),
        status_label: status.label().to_string(),
        status,
        reviews,
    }
}

fn fixture_report(cat: &Catalog) -> Report {
    Report {
        gates: Gates {
            is_technical_request: 0.95,
            has_enough_context: 0.4,
            injection: 0.1,
        },
        decisions: vec![
            cloud_platform_decision(cat),
            message_queue_decision(cat),
            relational_db_decision(cat),
        ],
        not_applicable: vec![NotApplicableView {
            type_id: "document-db".to_string(),
            type_name: "Document DB".to_string(),
            probability: 0.2,
        }],
        input_tokens: 12_000,
        cost_usd: Some(3.456),
        truncated: true,
        criteria: cat
            .criteria
            .iter()
            .map(|c| CriterionView {
                id: c.id.clone(),
                name: c.name.clone(),
                weight: c.weight,
            })
            .collect(),
    }
}

fn render_ctx<'a>(brief: &'a Brief, report: &'a Report) -> RenderContext<'a> {
    RenderContext {
        title: "Claims Portal Assessment",
        brief,
        report,
        cloud_choice: Some("Microsoft Azure"),
        today: NaiveDate::from_ymd_opt(2026, 9, 23).unwrap(),
    }
}

#[test]
fn golden_adr_for_accepted_adopt_decision() {
    let cat = Catalog::bundled().expect("bundled catalogue should load");
    let brief = fixture_brief();
    let report = fixture_report(&cat);
    let ctx = render_ctx(&brief, &report);

    let rendered =
        render_adr(&cat, &ctx, &report.decisions[0], 1).expect("render_adr should succeed");
    assert_matches_golden("adr-001-cloud-platform.md", &rendered);
}

#[test]
fn golden_adr_for_overridden_trial_decision() {
    let cat = Catalog::bundled().expect("bundled catalogue should load");
    let brief = fixture_brief();
    let report = fixture_report(&cat);
    let ctx = render_ctx(&brief, &report);

    let rendered =
        render_adr(&cat, &ctx, &report.decisions[1], 2).expect("render_adr should succeed");

    // Status states the overriding option, and the Decision section carries
    // the reviewer's option and reason (spec FR-19's Review-aware sections).
    assert!(rendered.contains("Accepted (override) — Azure Queue Storage"));
    assert!(rendered.contains(
    "The reviewer overrode this decision in favor of **Azure Queue Storage**: Team already runs Azure Queue Storage elsewhere; avoid new infra."
  ));

    assert_matches_golden("adr-002-message-queue-override.md", &rendered);
}

#[test]
fn golden_adr_for_unreviewed_hold_decision() {
    let cat = Catalog::bundled().expect("bundled catalogue should load");
    let brief = fixture_brief();
    let report = fixture_report(&cat);
    let ctx = render_ctx(&brief, &report);

    let rendered =
        render_adr(&cat, &ctx, &report.decisions[2], 3).expect("render_adr should succeed");

    // AC-14: unreviewed decisions export with this exact status line.
    assert!(rendered.contains("## Status\nProposed (AI) — pending approval"));
    // AC-14: Cost Analysis always carries this literal line.
    assert!(rendered.contains("Monthly estimate: to be completed by architect"));
    // No reviews recorded for this decision.
    assert!(rendered.contains("No reviews yet"));

    assert_matches_golden("adr-003-relational-db-hold.md", &rendered);
}

#[test]
fn golden_report_md() {
    let cat = Catalog::bundled().expect("bundled catalogue should load");
    let brief = fixture_brief();
    let report = fixture_report(&cat);
    let ctx = render_ctx(&brief, &report);

    let rendered = render_report_md(&cat, &ctx).expect("render_report_md should succeed");
    assert_matches_golden("report.md", &rendered);
}

#[test]
fn adr_numbers_are_zero_padded_to_three_digits() {
    let cat = Catalog::bundled().expect("bundled catalogue should load");
    let brief = fixture_brief();
    let report = fixture_report(&cat);
    let ctx = render_ctx(&brief, &report);

    let rendered =
        render_adr(&cat, &ctx, &report.decisions[0], 7).expect("render_adr should succeed");
    assert!(rendered.starts_with("# ADR-007: Cloud platform — Microsoft Azure"));
}

#[test]
fn export_adrs_numbers_continue_after_existing_adr_files_and_ignore_non_prefixed_names() {
    let cat = Catalog::bundled().expect("bundled catalogue should load");
    let brief = fixture_brief();
    let report = fixture_report(&cat);
    let ctx = render_ctx(&brief, &report);

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("ADR-007-x.md"), "existing").unwrap();
    std::fs::write(dir.path().join("notes-ADR-999.md"), "not a numbered adr").unwrap();
    assert_eq!(next_adr_number(dir.path()).unwrap(), 8);

    let paths = export_adrs(&cat, &ctx, dir.path()).expect("export_adrs should succeed");
    assert_eq!(paths.len(), 3);
    assert!(paths[0]
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("ADR-008-"));
    assert!(paths[1]
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("ADR-009-"));
    assert!(paths[2]
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("ADR-010-"));

    for path in &paths {
        assert!(path.exists(), "{path:?} should have been written");
    }
}

#[test]
fn slugify_produces_the_expected_filenames() {
    assert_eq!(
        slugify("Cloud platform Microsoft Azure"),
        "cloud-platform-microsoft-azure"
    );
    assert_eq!(
        slugify("Message queue (simple) Redis Pub/Sub"),
        "message-queue-simple-redis-pub-sub"
    );
    assert_eq!(
        slugify("Relational DB (budget) MySQL"),
        "relational-db-budget-mysql"
    );
}

#[test]
fn report_html_has_no_script_and_no_external_references() {
    let cat = Catalog::bundled().expect("bundled catalogue should load");
    let brief = fixture_brief();
    let report = fixture_report(&cat);
    let ctx = render_ctx(&brief, &report);

    let html = render_report_html(&cat, &ctx).expect("render_report_html should succeed");

    assert!(
        !html.contains("<script"),
        "HTML report must not contain <script>"
    );
    assert!(
        !html.contains("http://"),
        "HTML report must not reference http://"
    );
    assert!(
        !html.contains("https://"),
        "HTML report must not reference https://"
    );

    // Self-contained: no external stylesheet/script/font/image sources at all.
    assert!(!html.contains("src=\"http"));
    assert!(!html.contains("href=\"http"));
    assert!(!html.contains("url(http"));

    // Print-friendly.
    assert!(html.contains("@media print"));
}

#[test]
fn report_html_escapes_user_supplied_brief_text() {
    let cat = Catalog::bundled().expect("bundled catalogue should load");
    let brief = fixture_brief();
    let report = fixture_report(&cat);
    let ctx = render_ctx(&brief, &report);

    let html = render_report_html(&cat, &ctx).expect("render_report_html should succeed");

    // The fixture brief's team_skills item literally contains "<b>C#</b>";
    // minijinja's HTML autoescaping must neutralize it.
    assert!(
        !html.contains("<b>C#</b>"),
        "raw <b> from brief text must be escaped"
    );
    assert!(html.contains("&lt;b&gt;C#&lt;&#x2f;b&gt;"));
}

#[test]
fn report_md_contains_cost_and_banners() {
    let cat = Catalog::bundled().expect("bundled catalogue should load");
    let brief = fixture_brief();
    let report = fixture_report(&cat);
    let ctx = render_ctx(&brief, &report);

    let md = render_report_md(&cat, &ctx).expect("render_report_md should succeed");
    assert!(md.contains("$3.46"));
    assert!(md.contains("Low context"));
    assert!(md.contains("Truncated"));
    assert!(
        !md.contains("Possible prompt injection"),
        "injection is below 0.3 in this fixture"
    );
}
