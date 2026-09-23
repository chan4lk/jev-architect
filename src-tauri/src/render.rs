//! ADR and report rendering and export (spec FR-19, FR-20; AC-14, AC-15).
//!
//! Everything here is a pure function of its inputs: no current-time calls
//! (`RenderContext::today` stands in for "now"), no network or Jev calls,
//! and stable ordering / fixed float formatting throughout, so the same
//! `Report` always renders to byte-identical output (spec NFR-5).
//!
//! ADR markdown is filled in from `catalog/adr-template.md` (`Catalog::adr_template`,
//! minijinja placeholders). The Markdown and HTML report templates are
//! embedded from `src-tauri/src/templates/*.j2`; the HTML template is
//! rendered under minijinja's `.html` autoescaping so any user-supplied text
//! (brief items, option names, reasons, …) is HTML-escaped (spec AC-15).

use std::fs;
use std::path::{Path, PathBuf};

use minijinja::{context, Environment};
use regex::Regex;
use serde::Serialize;
use thiserror::Error;

use crate::model::brief::{Brief, BriefItem};
use crate::model::catalog::{Catalog, Ring};
use crate::model::decision::{DecisionResult, ReasonCode, Route};
use crate::model::report::{DecisionView, OptionView, Report};
use crate::model::review::{AdrStatus, Review, ReviewAction};

const REPORT_MD_TEMPLATE: &str = include_str!("templates/report.md.j2");
const REPORT_HTML_TEMPLATE: &str = include_str!("templates/report.html.j2");

const UNCALIBRATED_NOTE: &str =
  "Thresholds and weights are uncalibrated defaults until the golden-set run (FR-17) has been done against the live API.";

/// Errors from rendering or exporting an ADR or a report (spec FR-19, FR-20).
#[derive(Debug, Error)]
pub enum RenderError {
    #[error("template error: {0}")]
    Template(#[from] minijinja::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Everything a render needs beyond the catalogue: the session title, the
/// confirmed brief, the assembled report, the cloud-platform decision's
/// chosen option name (if any), and "today" (spec: no current-time calls
/// inside render, so the caller supplies it).
pub struct RenderContext<'a> {
    pub title: &'a str,
    pub brief: &'a Brief,
    pub report: &'a Report,
    pub cloud_choice: Option<&'a str>,
    pub today: chrono::NaiveDate,
}

/// The export format for [`export_report`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportFormat {
    Md,
    Html,
}

// ─────────────────────────────────────────────────────────────────────────
// ADR rendering
// ─────────────────────────────────────────────────────────────────────────

/// Renders one decision as an ADR markdown document (spec FR-19), using the
/// `bistec-architect` ADR template (`cat.adr_template`).
pub fn render_adr(
    cat: &Catalog,
    ctx: &RenderContext,
    view: &DecisionView,
    number: u32,
) -> Result<String, RenderError> {
    let chosen = find_option(view, &view.decision.choice);
    let chosen_name = chosen
        .map(|o| o.name.as_str())
        .unwrap_or(&view.decision.choice);
    let chosen_ring = chosen.map(|o| o.ring);

    let title = format!("{} — {}", view.type_name, chosen_name);
    let number_str = format!("{number:03}");
    let date = ctx.today.format("%Y-%m-%d").to_string();

    let status_section = render_status(view);
    let context_section = render_context_section(ctx.brief, &view.decision);
    let decision_section = render_decision_section(view, chosen_name, chosen_ring);
    let alignment_default = alignment_default_text(chosen_ring);
    let alignment_cloud = ctx.cloud_choice.unwrap_or("n/a").to_string();
    let alignment_cost = alignment_cost_text(ctx.brief, view);
    let alternatives = render_alternatives_table(cat, view);
    let consequences = render_consequences(cat, view);
    let cost_analysis = render_cost_analysis(ctx.brief, view);
    let review_section = render_review_table(view);
    let evidence = render_evidence(ctx, &view.decision);

    let env = Environment::new();
    let rendered = env.render_named_str(
        "adr.md",
        &cat.adr_template,
        context! {
          number => number_str,
          title => title,
          status => status_section,
          date => date,
          context => context_section,
          decision => decision_section,
          alignment_default => alignment_default,
          alignment_cloud => alignment_cloud,
          alignment_cost => alignment_cost,
          alternatives => alternatives,
          consequences => consequences,
          cost_analysis => cost_analysis,
          review => review_section,
          evidence => evidence,
        },
    )?;
    Ok(rendered)
}

/// Numbers the next ADR export in `dir` (spec FR-19). Scans file names for
/// `^ADR-(\d{3,})-` (only those count towards numbering; a name like
/// `notes-ADR-999.md` does not start with the prefix and is ignored), and
/// returns the highest number found, plus one — or `1` if none, or if `dir`
/// does not exist yet.
pub fn next_adr_number(dir: &Path) -> Result<u32, RenderError> {
    let re = Regex::new(r"^ADR-(\d{3,})-").expect("valid regex");
    let mut max = 0u32;

    if dir.is_dir() {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if let Some(caps) = re.captures(&name) {
                if let Ok(n) = caps[1].parse::<u32>() {
                    max = max.max(n);
                }
            }
        }
    }

    Ok(max + 1)
}

/// Slugifies `s` for use in a filename: lowercased, non-alphanumeric runs
/// collapsed to a single `-`, with no leading or trailing `-`.
pub fn slugify(s: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;

    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }

    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// Exports every decision in `ctx.report.decisions` (in that order) as an
/// ADR markdown file in `dir` (spec FR-19). Numbers are assigned
/// sequentially starting from [`next_adr_number`], i.e. the `i`-th returned
/// path (0-indexed) is `ADR-{n}` where `n = next_adr_number(dir) + i`.
/// Files are named `ADR-NNN-<slug of type name and chosen option>.md`.
pub fn export_adrs(
    cat: &Catalog,
    ctx: &RenderContext,
    dir: &Path,
) -> Result<Vec<PathBuf>, RenderError> {
    fs::create_dir_all(dir)?;
    let mut number = next_adr_number(dir)?;
    let mut paths = Vec::with_capacity(ctx.report.decisions.len());

    for view in &ctx.report.decisions {
        let rendered = render_adr(cat, ctx, view, number)?;

        let chosen_name = find_option(view, &view.decision.choice)
            .map(|o| o.name.as_str())
            .unwrap_or(&view.decision.choice);
        let slug = slugify(&format!("{} {}", view.type_name, chosen_name));
        let filename = format!("ADR-{number:03}-{slug}.md");

        let path = dir.join(filename);
        fs::write(&path, rendered)?;
        paths.push(path);
        number += 1;
    }

    Ok(paths)
}

// ─────────────────────────────────────────────────────────────────────────
// Report rendering
// ─────────────────────────────────────────────────────────────────────────

/// Renders the whole report as Markdown (spec FR-20).
pub fn render_report_md(cat: &Catalog, ctx: &RenderContext) -> Result<String, RenderError> {
    let vm = build_report_vm(cat, ctx);
    let env = Environment::new();
    Ok(env.render_named_str("report.md", REPORT_MD_TEMPLATE, &vm)?)
}

/// Renders the whole report as a single self-contained HTML document (spec
/// FR-20, AC-15): inline `<style>` only, no `<script>`, and no
/// `http(s)://`-referenced stylesheet/script/font/image. The template name
/// ends in `.html`, which turns on minijinja's autoescaping, so every
/// user-supplied string (brief text, option names, reasons, …) is
/// HTML-escaped.
pub fn render_report_html(cat: &Catalog, ctx: &RenderContext) -> Result<String, RenderError> {
    let vm = build_report_vm(cat, ctx);
    let env = Environment::new();
    Ok(env.render_named_str("report.html", REPORT_HTML_TEMPLATE, &vm)?)
}

/// Exports the report in `format` to `dir` (spec FR-20), as
/// `<slug(title)>-report.md` or `<slug(title)>-report.html`.
pub fn export_report(
    cat: &Catalog,
    ctx: &RenderContext,
    dir: &Path,
    format: ReportFormat,
) -> Result<PathBuf, RenderError> {
    fs::create_dir_all(dir)?;

    let (content, ext) = match format {
        ReportFormat::Md => (render_report_md(cat, ctx)?, "md"),
        ReportFormat::Html => (render_report_html(cat, ctx)?, "html"),
    };

    let filename = format!("{}-report.{ext}", slugify(ctx.title));
    let path = dir.join(filename);
    fs::write(&path, content)?;
    Ok(path)
}

// ─────────────────────────────────────────────────────────────────────────
// Shared formatting helpers
// ─────────────────────────────────────────────────────────────────────────

fn find_option<'a>(view: &'a DecisionView, option_id: &str) -> Option<&'a OptionView> {
    view.options.iter().find(|o| o.id == option_id)
}

fn ring_word(ring: Ring) -> &'static str {
    match ring {
        Ring::Adopt => "Adopt",
        Ring::Trial => "Trial",
        Ring::Hold => "Hold",
    }
}

fn ring_class(ring: Ring) -> &'static str {
    match ring {
        Ring::Adopt => "adopt",
        Ring::Trial => "trial",
        Ring::Hold => "hold",
    }
}

fn route_word(route: Route) -> &'static str {
    match route {
        Route::Proposed => "Proposed",
        Route::NeedsArchitect => "Needs architect",
    }
}

fn reason_word(reason: &ReasonCode) -> &'static str {
    match reason {
        ReasonCode::LowConfidence => "low confidence",
        ReasonCode::Disagreement => "disagreement",
        ReasonCode::CloseMargin => "close margin",
        ReasonCode::HoldOption => "hold option",
        ReasonCode::PossibleInjection => "possible injection",
    }
}

fn reason_words(reasons: &[ReasonCode]) -> String {
    if reasons.is_empty() {
        return "none".to_string();
    }
    reasons
        .iter()
        .map(reason_word)
        .collect::<Vec<_>>()
        .join(", ")
}

fn action_word(action: ReviewAction) -> &'static str {
    match action {
        ReviewAction::Accept => "Accept",
        ReviewAction::Override => "Override",
        ReviewAction::Reject => "Reject",
    }
}

/// The latest review by `(at_utc, id)`, matching `review::adr_status`'s own
/// tie-break rule (a higher id wins a same-timestamp tie).
fn latest_review(reviews: &[Review]) -> Option<&Review> {
    reviews.iter().max_by_key(|r| (r.at_utc, r.id))
}

fn review_option_name(view: &DecisionView, review: &Review) -> String {
    review
        .option_id
        .as_deref()
        .map(|id| {
            find_option(view, id)
                .map(|o| o.name.clone())
                .unwrap_or_else(|| id.to_string())
        })
        .unwrap_or_else(|| "—".to_string())
}

fn cost_fit_score(decision: &DecisionResult, option_id: &str) -> Option<f64> {
    decision
        .option_scores
        .iter()
        .find(|s| s.option_id == option_id)
        .and_then(|s| s.criterion_scores.get("cost_fit").copied())
}

fn compliance_text(brief: &Brief) -> String {
    if brief.context.compliance.is_empty() {
        "Not stated in the evidence.".to_string()
    } else {
        brief
            .context
            .compliance
            .iter()
            .map(|c| c.describe())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

fn items_text(items: &[BriefItem]) -> Vec<String> {
    items
        .iter()
        .map(|i| {
            if i.sources.is_empty() {
                i.text.clone()
            } else {
                format!("{} ({})", i.text, i.sources.join(", "))
            }
        })
        .collect()
}

fn bullet_list(items: &[BriefItem]) -> String {
    let lines = items_text(items);
    if lines.is_empty() {
        "- None stated.".to_string()
    } else {
        lines
            .iter()
            .map(|l| format!("- {l}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn joined_or(items: &[String], empty: &str) -> String {
    if items.is_empty() {
        empty.to_string()
    } else {
        items.join("; ")
    }
}

// ─────────────────────────────────────────────────────────────────────────
// ADR section builders
// ─────────────────────────────────────────────────────────────────────────

fn render_status(view: &DecisionView) -> String {
    if view.status == AdrStatus::AcceptedOverride {
        if let Some(r) = latest_review(&view.reviews) {
            return format!("{} — {}", view.status_label, review_option_name(view, r));
        }
    }
    view.status_label.clone()
}

fn render_context_section(brief: &Brief, decision: &DecisionResult) -> String {
    let ctx = &brief.context;
    let cited = if decision.cited_sections.is_empty() {
        "None".to_string()
    } else {
        decision.cited_sections.join(", ")
    };

    format!(
        "{summary}\n\n\
     | Dimension | Value |\n\
     |---|---|\n\
     | Scale | {scale} |\n\
     | Budget | {budget} |\n\
     | Timeline | {timeline} |\n\
     | Team size | {team_size} |\n\
     | Compliance | {compliance} |\n\
     | Data sensitivity | {data_sensitivity} |\n\n\
     **Constraints:**\n{constraints}\n\n\
     **Non-functional requirements:**\n{nfrs}\n\n\
     **Cited sections:** {cited}",
        summary = brief.summary,
        scale = ctx.scale.describe(),
        budget = ctx.budget.describe(),
        timeline = ctx.timeline.describe(),
        team_size = ctx.team_size.describe(),
        compliance = compliance_text(brief),
        data_sensitivity = ctx.data_sensitivity.describe(),
        constraints = bullet_list(&brief.constraints),
        nfrs = bullet_list(&brief.nfrs),
        cited = cited,
    )
}

fn render_decision_section(
    view: &DecisionView,
    chosen_name: &str,
    chosen_ring: Option<Ring>,
) -> String {
    let d = &view.decision;
    let ring = chosen_ring.map(ring_word).unwrap_or("Unknown");
    let probability = d.probabilities.get(&d.choice).copied().unwrap_or(0.0);

    let mut section = format!(
    "The chosen option is **{chosen_name}** ({ring}), with a Jev probability of {probability:.2} and confidence {confidence:.2}.\n\n\
     Route: **{route}**. Reason codes: {reasons}.",
    route = route_word(d.route),
    reasons = reason_words(&d.reasons),
    confidence = d.confidence,
  );

    if view.status == AdrStatus::AcceptedOverride {
        if let Some(r) = latest_review(&view.reviews) {
            let overriding_name = review_option_name(view, r);
            let reason = r.reason.as_deref().unwrap_or("");
            section.push_str(&format!(
        "\n\nThe reviewer overrode this decision in favor of **{overriding_name}**: {reason}"
      ));
        }
    }

    section
}

fn alignment_default_text(ring: Option<Ring>) -> String {
    match ring {
        Some(Ring::Adopt) => "Follows the BISTEC default".to_string(),
        Some(Ring::Trial) => {
            "Deviation: BISTEC alternative — rationale required (see Review)".to_string()
        }
        Some(Ring::Hold) => "Breach: BISTEC says avoid — architect approval required".to_string(),
        None => "Unknown".to_string(),
    }
}

fn alignment_cost_text(brief: &Brief, view: &DecisionView) -> String {
    let band = brief.context.budget.describe();
    match cost_fit_score(&view.decision, &view.decision.choice) {
        Some(v) => format!("{band} — cost fit score: {v:.2}/4"),
        None => format!("{band} — cost fit score: —"),
    }
}

fn render_alternatives_table(cat: &Catalog, view: &DecisionView) -> String {
    let criteria = &cat.criteria;

    let mut header_cells = vec!["Option".to_string(), "Ring".to_string()];
    header_cells.extend(criteria.iter().map(|c| c.name.clone()));
    header_cells.push("Composite".to_string());
    header_cells.push("Probability".to_string());

    let header = format!("| {} |", header_cells.join(" | "));
    let sep = format!(
        "|{}|",
        header_cells
            .iter()
            .map(|_| "---")
            .collect::<Vec<_>>()
            .join("|")
    );

    let mut lines = vec![header, sep];
    for opt in &view.options {
        let scores = view
            .decision
            .option_scores
            .iter()
            .find(|s| s.option_id == opt.id);

        let mut cells = vec![opt.name.clone(), ring_word(opt.ring).to_string()];
        for c in criteria {
            let cell = scores
                .and_then(|s| s.criterion_scores.get(&c.id))
                .map(|v| format!("{v:.2}"))
                .unwrap_or_else(|| "—".to_string());
            cells.push(cell);
        }
        cells.push(
            scores
                .map(|s| format!("{:.3}", s.composite))
                .unwrap_or_else(|| "—".to_string()),
        );
        cells.push(
            view.decision
                .probabilities
                .get(&opt.id)
                .map(|v| format!("{v:.2}"))
                .unwrap_or_else(|| "—".to_string()),
        );

        lines.push(format!("| {} |", cells.join(" | ")));
    }

    lines.join("\n")
}

fn render_consequences(cat: &Catalog, view: &DecisionView) -> String {
    let d = &view.decision;
    let scores = d.option_scores.iter().find(|s| s.option_id == d.choice);

    let mut positive = Vec::new();
    let mut negative = Vec::new();
    let mut low_criteria = Vec::new();

    if let Some(scores) = scores {
        for c in &cat.criteria {
            if let Some(v) = scores.criterion_scores.get(&c.id) {
                if *v >= 3.0 {
                    positive.push(format!("{} ({v:.2}/4)", c.name));
                }
                if *v <= 1.0 {
                    negative.push(format!("{} ({v:.2}/4)", c.name));
                    low_criteria.push(format!("low score in {} ({v:.2}/4)", c.name));
                }
            }
        }
    }

    let mut risks: Vec<String> = d
        .reasons
        .iter()
        .map(|r| reason_word(r).to_string())
        .collect();
    risks.extend(low_criteria);

    format!(
        "- Positive: {}\n- Negative: {}\n- Risks: {}",
        joined_or(&positive, "None"),
        joined_or(&negative, "None"),
        joined_or(&risks, "None"),
    )
}

fn render_cost_analysis(brief: &Brief, view: &DecisionView) -> String {
    let mut lines = vec![
        format!("Budget band: {}", brief.context.budget.describe()),
        String::new(),
        "| Option | Cost fit |".to_string(),
        "|---|---|".to_string(),
    ];

    for opt in &view.options {
        let cell = cost_fit_score(&view.decision, &opt.id)
            .map(|v| format!("{v:.2}/4"))
            .unwrap_or_else(|| "—".to_string());
        lines.push(format!("| {} | {} |", opt.name, cell));
    }

    lines.push(String::new());
    lines.push("Monthly estimate: to be completed by architect".to_string());
    lines.join("\n")
}

fn render_review_table(view: &DecisionView) -> String {
    if view.reviews.is_empty() {
        return "No reviews yet".to_string();
    }

    let mut reviews: Vec<&Review> = view.reviews.iter().collect();
    reviews.sort_by_key(|r| (r.at_utc, r.id));

    let mut lines = vec![
        "| Time (UTC) | Reviewer | Action | Option | Reason |".to_string(),
        "|---|---|---|---|---|".to_string(),
    ];
    for r in reviews {
        lines.push(format!(
            "| {} | {} | {} | {} | {} |",
            r.at_utc.format("%Y-%m-%dT%H:%M:%SZ"),
            r.reviewer,
            action_word(r.action),
            review_option_name(view, r),
            r.reason.as_deref().unwrap_or("—"),
        ));
    }

    lines.join("\n")
}

fn render_evidence(ctx: &RenderContext, decision: &DecisionResult) -> String {
    format!(
    "- Model snapshot: {}\n- Request hash: {}\n- Decision id: {}\n- Gates: is_technical_request={:.2}, has_enough_context={:.2}, injection={:.2}",
    decision.model_snapshot,
    decision.request_hash,
    decision.id,
    ctx.report.gates.is_technical_request,
    ctx.report.gates.has_enough_context,
    ctx.report.gates.injection,
  )
}

// ─────────────────────────────────────────────────────────────────────────
// Report view model (shared by render_report_md / render_report_html)
// ─────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
struct BriefVm {
    summary: String,
    scale: String,
    budget: String,
    timeline: String,
    team_size: String,
    compliance: String,
    data_sensitivity: String,
    requirements: Vec<String>,
    nfrs: Vec<String>,
    constraints: Vec<String>,
    team_skills: Vec<String>,
    mentioned_technologies: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
struct SummaryVm {
    total: usize,
    proposed: usize,
    needs_architect: usize,
    accepted: usize,
    overridden: usize,
    rejected: usize,
    unreviewed: usize,
    cost: String,
    input_tokens: u64,
    low_context: bool,
    truncated: bool,
    injection_flag: bool,
}

#[derive(Debug, Clone, Serialize)]
struct DecisionRowVm {
    type_name: String,
    choice_name: String,
    ring_word: String,
    ring_class: String,
    confidence: String,
    route_word: String,
    status_label: String,
}

#[derive(Debug, Clone, Serialize)]
struct OptionRowVm {
    option_name: String,
    ring_word: String,
    ring_class: String,
    probability: String,
    probability_pct: u32,
    scores: Vec<String>,
    composite: String,
}

#[derive(Debug, Clone, Serialize)]
struct DecisionSectionVm {
    type_name: String,
    choice_name: String,
    ring_word: String,
    ring_class: String,
    confidence: String,
    route_word: String,
    reasons: Vec<String>,
    model_snapshot: String,
    cited_sections: Vec<String>,
    options: Vec<OptionRowVm>,
}

#[derive(Debug, Clone, Serialize)]
struct NotApplicableVm {
    type_name: String,
    probability: String,
}

#[derive(Debug, Clone, Serialize)]
struct ReportVm {
    title: String,
    date: String,
    summary: SummaryVm,
    brief: BriefVm,
    criteria_names: Vec<String>,
    decisions_table: Vec<DecisionRowVm>,
    decision_sections: Vec<DecisionSectionVm>,
    not_applicable: Vec<NotApplicableVm>,
    uncalibrated_note: String,
}

fn build_report_vm(cat: &Catalog, ctx: &RenderContext) -> ReportVm {
    let brief = ctx.brief;
    let report = ctx.report;

    let brief_vm = BriefVm {
        summary: brief.summary.clone(),
        scale: brief.context.scale.describe().to_string(),
        budget: brief.context.budget.describe().to_string(),
        timeline: brief.context.timeline.describe().to_string(),
        team_size: brief.context.team_size.describe().to_string(),
        compliance: compliance_text(brief),
        data_sensitivity: brief.context.data_sensitivity.describe().to_string(),
        requirements: items_text(&brief.requirements),
        nfrs: items_text(&brief.nfrs),
        constraints: items_text(&brief.constraints),
        team_skills: items_text(&brief.team_skills),
        mentioned_technologies: brief.mentioned_technologies.clone(),
    };

    let mut proposed = 0usize;
    let mut needs_architect = 0usize;
    let mut accepted = 0usize;
    let mut overridden = 0usize;
    let mut rejected = 0usize;
    let mut unreviewed = 0usize;

    for d in &report.decisions {
        match d.decision.route {
            Route::Proposed => proposed += 1,
            Route::NeedsArchitect => needs_architect += 1,
        }
        match d.status {
            AdrStatus::ProposedAi => unreviewed += 1,
            AdrStatus::Accepted => accepted += 1,
            AdrStatus::AcceptedOverride => overridden += 1,
            AdrStatus::Rejected => rejected += 1,
        }
    }

    let summary_vm = SummaryVm {
        total: report.decisions.len(),
        proposed,
        needs_architect,
        accepted,
        overridden,
        rejected,
        unreviewed,
        cost: report
            .cost_usd
            .map(|c| format!("${c:.2}"))
            .unwrap_or_else(|| "n/a".to_string()),
        input_tokens: report.input_tokens,
        low_context: report.gates.has_enough_context < 0.5,
        truncated: report.truncated,
        injection_flag: report.gates.injection >= 0.3,
    };

    let criteria_names: Vec<String> = cat.criteria.iter().map(|c| c.name.clone()).collect();

    let decisions_table: Vec<DecisionRowVm> = report
        .decisions
        .iter()
        .map(|d| {
            let chosen = find_option(d, &d.decision.choice);
            DecisionRowVm {
                type_name: d.type_name.clone(),
                choice_name: chosen
                    .map(|o| o.name.clone())
                    .unwrap_or_else(|| d.decision.choice.clone()),
                ring_word: chosen
                    .map(|o| ring_word(o.ring).to_string())
                    .unwrap_or_else(|| "—".to_string()),
                ring_class: chosen
                    .map(|o| ring_class(o.ring).to_string())
                    .unwrap_or_default(),
                confidence: format!("{:.2}", d.decision.confidence),
                route_word: route_word(d.decision.route).to_string(),
                status_label: d.status_label.clone(),
            }
        })
        .collect();

    let decision_sections: Vec<DecisionSectionVm> = report
        .decisions
        .iter()
        .map(|d| build_decision_section_vm(cat, d))
        .collect();

    let not_applicable: Vec<NotApplicableVm> = report
        .not_applicable
        .iter()
        .map(|n| NotApplicableVm {
            type_name: n.type_name.clone(),
            probability: format!("{:.2}", n.probability),
        })
        .collect();

    ReportVm {
        title: ctx.title.to_string(),
        date: ctx.today.format("%Y-%m-%d").to_string(),
        summary: summary_vm,
        brief: brief_vm,
        criteria_names,
        decisions_table,
        decision_sections,
        not_applicable,
        uncalibrated_note: UNCALIBRATED_NOTE.to_string(),
    }
}

fn build_decision_section_vm(cat: &Catalog, view: &DecisionView) -> DecisionSectionVm {
    let d = &view.decision;
    let chosen = find_option(view, &d.choice);

    let options: Vec<OptionRowVm> = view
        .options
        .iter()
        .map(|opt| {
            let scores = d.option_scores.iter().find(|s| s.option_id == opt.id);
            let probability = d.probabilities.get(&opt.id).copied();

            let score_cells: Vec<String> = cat
                .criteria
                .iter()
                .map(|c| {
                    scores
                        .and_then(|s| s.criterion_scores.get(&c.id))
                        .map(|v| format!("{v:.2}"))
                        .unwrap_or_else(|| "—".to_string())
                })
                .collect();

            OptionRowVm {
                option_name: opt.name.clone(),
                ring_word: ring_word(opt.ring).to_string(),
                ring_class: ring_class(opt.ring).to_string(),
                probability: probability
                    .map(|p| format!("{p:.2}"))
                    .unwrap_or_else(|| "—".to_string()),
                probability_pct: probability
                    .map(|p| (p.clamp(0.0, 1.0) * 100.0).round() as u32)
                    .unwrap_or(0),
                scores: score_cells,
                composite: scores
                    .map(|s| format!("{:.3}", s.composite))
                    .unwrap_or_else(|| "—".to_string()),
            }
        })
        .collect();

    DecisionSectionVm {
        type_name: view.type_name.clone(),
        choice_name: chosen
            .map(|o| o.name.clone())
            .unwrap_or_else(|| d.choice.clone()),
        ring_word: chosen
            .map(|o| ring_word(o.ring).to_string())
            .unwrap_or_else(|| "—".to_string()),
        ring_class: chosen
            .map(|o| ring_class(o.ring).to_string())
            .unwrap_or_default(),
        confidence: format!("{:.2}", d.confidence),
        route_word: route_word(d.route).to_string(),
        reasons: d
            .reasons
            .iter()
            .map(|r| reason_word(r).to_string())
            .collect(),
        model_snapshot: d.model_snapshot.clone(),
        cited_sections: d.cited_sections.clone(),
        options,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_lowercases_and_collapses_separators() {
        assert_eq!(slugify("Node.js / TypeScript"), "node-js-typescript");
        assert_eq!(slugify(".NET 8+"), "net-8");
        assert_eq!(slugify("  leading and trailing  "), "leading-and-trailing");
        assert_eq!(slugify("Already-Slugged"), "already-slugged");
    }

    #[test]
    fn next_adr_number_is_one_when_dir_is_missing_or_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(next_adr_number(dir.path()).unwrap(), 1);

        let missing = dir.path().join("does-not-exist");
        assert_eq!(next_adr_number(&missing).unwrap(), 1);
    }

    #[test]
    fn next_adr_number_continues_after_existing_files_and_ignores_non_prefixed_names() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("ADR-007-x.md"), "x").unwrap();
        fs::write(dir.path().join("notes-ADR-999.md"), "x").unwrap();
        assert_eq!(next_adr_number(dir.path()).unwrap(), 8);
    }

    #[test]
    fn reason_words_lists_every_reason_in_order() {
        assert_eq!(reason_words(&[]), "none");
        assert_eq!(
            reason_words(&[ReasonCode::LowConfidence, ReasonCode::CloseMargin]),
            "low confidence, close margin"
        );
    }
}
