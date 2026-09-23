import { z } from "zod";

// ---------------------------------------------------------------------------
// Shared enums / literal unions (see .specclaw/changes/001-bistec-architect-agent/ipc-contract.md)
// ---------------------------------------------------------------------------

export const RingSchema = z.enum(["adopt", "trial", "hold"]);
export type Ring = z.infer<typeof RingSchema>;

export const RouteSchema = z.enum(["proposed", "needs_architect"]);
export type Route = z.infer<typeof RouteSchema>;

export const ReasonCodeSchema = z.enum([
  "low_confidence",
  "disagreement",
  "close_margin",
  "hold_option",
  "possible_injection",
]);
export type ReasonCode = z.infer<typeof ReasonCodeSchema>;

export const AdrStatusSchema = z.enum([
  "proposed_ai",
  "accepted",
  "accepted_override",
  "rejected",
]);
export type AdrStatus = z.infer<typeof AdrStatusSchema>;

export const ReviewActionSchema = z.enum(["accept", "override", "reject"]);
export type ReviewAction = z.infer<typeof ReviewActionSchema>;

export const ErrorCodeSchema = z.enum([
  "no_api_key",
  "data_notice_required",
  "not_found",
  "validation",
  "brief_extraction_failed",
  "jev",
  "local_model",
  "document",
  "store",
  "io",
  "out_of_remit",
  "internal",
]);
export type ErrorCode = z.infer<typeof ErrorCodeSchema>;

export const AppErrorSchema = z.object({
  code: ErrorCodeSchema,
  message: z.string(),
  stage: z.string().optional(),
});
export type AppErrorShape = z.infer<typeof AppErrorSchema>;

/** Thrown by both the mock and real IPC implementations for every rejected command. */
export class AppError extends Error {
  code: ErrorCode;
  stage?: string;

  constructor(shape: AppErrorShape) {
    super(shape.message);
    this.name = "AppError";
    this.code = shape.code;
    this.stage = shape.stage;
  }
}

// ---------------------------------------------------------------------------
// Settings / criteria
// ---------------------------------------------------------------------------

export const SettingsSchema = z.object({
  jev_model: z.string(),
  jev_base_url: z.string(),
  ollama_base_url: z.string(),
  ollama_model: z.string(),
  state_token_budget: z.number(),
  confidence_threshold: z.number(),
  min_margin: z.number(),
  max_questions_per_call: z.number(),
  max_concurrent_calls: z.number(),
  weights: z.record(z.string(), z.number()),
  reviewer_name: z.string(),
});
export type Settings = z.infer<typeof SettingsSchema>;

export const CriterionSchema = z.object({
  id: z.string(),
  name: z.string(),
  weight: z.number(),
});
export type Criterion = z.infer<typeof CriterionSchema>;

// ---------------------------------------------------------------------------
// Brief
// ---------------------------------------------------------------------------

export const ScaleSchema = z.enum(["small", "medium", "large", "unknown"]);
export const BudgetSchema = z.enum(["tight", "moderate", "enterprise", "unknown"]);
export const TimelineSchema = z.enum(["urgent", "normal", "long_term", "unknown"]);
export const TeamSizeSchema = z.enum(["solo_pair", "small", "large", "unknown"]);
export const ComplianceSchema = z.enum(["soc2", "gdpr", "hipaa", "industry_specific"]);
export const DataSensitivitySchema = z.enum([
  "public",
  "internal",
  "confidential",
  "restricted",
  "unknown",
]);

export const ContextAssessmentSchema = z.object({
  scale: ScaleSchema,
  budget: BudgetSchema,
  timeline: TimelineSchema,
  team_size: TeamSizeSchema,
  compliance: z.array(ComplianceSchema),
  data_sensitivity: DataSensitivitySchema,
});
export type ContextAssessment = z.infer<typeof ContextAssessmentSchema>;

export const BriefItemSchema = z.object({
  text: z.string(),
  sources: z.array(z.string()),
});
export type BriefItem = z.infer<typeof BriefItemSchema>;

export const BriefSchema = z.object({
  summary: z.string(),
  context: ContextAssessmentSchema,
  requirements: z.array(BriefItemSchema),
  nfrs: z.array(BriefItemSchema),
  constraints: z.array(BriefItemSchema),
  team_skills: z.array(BriefItemSchema),
  mentioned_technologies: z.array(z.string()),
});
export type Brief = z.infer<typeof BriefSchema>;

export const SectionSchema = z.object({
  id: z.string(),
  heading: z.string().nullable(),
  text: z.string(),
  tokens: z.number(),
});
export type Section = z.infer<typeof SectionSchema>;

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

export const SessionModeSchema = z.enum(["describe", "upload"]);
export type SessionMode = z.infer<typeof SessionModeSchema>;

export const SessionSummarySchema = z.object({
  id: z.string(),
  created_at: z.string(),
  title: z.string(),
  mode: SessionModeSchema,
  stage: z.string(),
  decision_count: z.number(),
  needs_architect_count: z.number(),
  unreviewed_count: z.number(),
});
export type SessionSummary = z.infer<typeof SessionSummarySchema>;

export const SessionSchema = z.object({
  id: z.string(),
  created_at: z.string(),
  title: z.string(),
  mode: SessionModeSchema,
  input_text: z.string().nullable(),
  doc_name: z.string().nullable(),
  stage: z.string(),
  error: z.string().nullable(),
  brief_confirmed_at: z.string().nullable(),
});
export type Session = z.infer<typeof SessionSchema>;

// ---------------------------------------------------------------------------
// Decisions / report
// ---------------------------------------------------------------------------

export const OptionViewSchema = z.object({
  id: z.string(),
  name: z.string(),
  ring: RingSchema,
  description: z.string(),
});
export type OptionView = z.infer<typeof OptionViewSchema>;

export const OptionScoreSchema = z.object({
  option_id: z.string(),
  criterion_scores: z.record(z.string(), z.number()),
  composite: z.number(),
});
export type OptionScore = z.infer<typeof OptionScoreSchema>;

export const DecisionResultSchema = z.object({
  id: z.string(),
  session_id: z.string(),
  type_id: z.string(),
  choice: z.string(),
  confidence: z.number(),
  probabilities: z.record(z.string(), z.number()),
  option_scores: z.array(OptionScoreSchema),
  route: RouteSchema,
  reasons: z.array(ReasonCodeSchema),
  model_snapshot: z.string(),
  request_hash: z.string(),
  cited_sections: z.array(z.string()),
});
export type DecisionResult = z.infer<typeof DecisionResultSchema>;

export const ReviewSchema = z.object({
  id: z.number(),
  decision_id: z.string(),
  action: ReviewActionSchema,
  option_id: z.string().nullable(),
  reviewer: z.string(),
  reason: z.string().nullable(),
  at_utc: z.string(),
});
export type Review = z.infer<typeof ReviewSchema>;

export const DecisionViewSchema = z.object({
  decision: DecisionResultSchema,
  type_name: z.string(),
  options: z.array(OptionViewSchema),
  reviews: z.array(ReviewSchema),
  status: AdrStatusSchema,
  status_label: z.string(),
});
export type DecisionView = z.infer<typeof DecisionViewSchema>;

export const NotApplicableSchema = z.object({
  type_id: z.string(),
  type_name: z.string(),
  probability: z.number(),
});
export type NotApplicable = z.infer<typeof NotApplicableSchema>;

export const GatesSchema = z.object({
  is_technical_request: z.number(),
  has_enough_context: z.number(),
  injection: z.number(),
});
export type Gates = z.infer<typeof GatesSchema>;

export const ReportSchema = z.object({
  gates: GatesSchema,
  decisions: z.array(DecisionViewSchema),
  not_applicable: z.array(NotApplicableSchema),
  input_tokens: z.number(),
  cost_usd: z.number().nullable(),
  truncated: z.boolean(),
  criteria: z.array(CriterionSchema),
});
export type Report = z.infer<typeof ReportSchema>;

export const DocPathSchema = z.enum(["small", "large"]);
export type DocPath = z.infer<typeof DocPathSchema>;

export const SessionViewSchema = z.object({
  session: SessionSchema,
  sections: z.array(SectionSchema).nullable(),
  doc_path: DocPathSchema.nullable(),
  brief: BriefSchema.nullable(),
  report: ReportSchema.nullable(),
});
export type SessionView = z.infer<typeof SessionViewSchema>;

// ---------------------------------------------------------------------------
// Health / progress / data notice
// ---------------------------------------------------------------------------

export const OpenRouterStatusSchema = z.enum(["ok", "no_key", "error"]);
export type OpenRouterStatus = z.infer<typeof OpenRouterStatusSchema>;

export const HealthSchema = z.object({
  ollama_reachable: z.boolean(),
  model_present: z.boolean(),
  model: z.string(),
  pull_command: z.string(),
  openrouter: OpenRouterStatusSchema,
  openrouter_error: z.string().nullable(),
});
export type Health = z.infer<typeof HealthSchema>;

/** The Ollama-only half of `Health`: no Jev call, cheap to poll on every Home load. */
export const LocalModelStatusSchema = z.object({
  ollama_reachable: z.boolean(),
  model_present: z.boolean(),
  model: z.string(),
  pull_command: z.string(),
});
export type LocalModelStatus = z.infer<typeof LocalModelStatusSchema>;

export const ProgressSchema = z.object({
  session_id: z.string(),
  stage: z.string(),
  done: z.number(),
  total: z.number(),
  message: z.string().nullable(),
});
export type Progress = z.infer<typeof ProgressSchema>;

export const DataNoticeSchema = z.object({
  text: z.string(),
  acked: z.boolean(),
});
export type DataNotice = z.infer<typeof DataNoticeSchema>;
