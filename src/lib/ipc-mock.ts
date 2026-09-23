/**
 * In-memory mock implementation of the IPC contract
 * (.specclaw/changes/001-bistec-architect-agent/ipc-contract.md), used when the
 * app is not running inside Tauri (plain browser, Vitest, Playwright e2e).
 *
 * `mockApi` implements the same surface as the real Tauri-backed implementation
 * in `ipc.ts`. `resetMock()` restores the initial seeded state and is meant to
 * be called from `beforeEach` in tests. `setHealthOverride()` is a test-only
 * hook for forcing `health_check()` results (e.g. simulating a missing model).
 *
 * T12/T13/T15 reuse this mock, so its seeded data is kept realistic and
 * reasonably complete rather than minimal.
 */
import {
  type AppErrorShape,
  type Brief,
  type Criterion,
  type DecisionView,
  type Health,
  type Progress,
  type ReasonCode,
  type Report,
  type Review,
  type ReviewAction,
  type Ring,
  type Route,
  type Section,
  type Session,
  type SessionMode,
  type SessionSummary,
  type SessionView,
  type Settings,
  AppError,
} from "./schemas";

// ---------------------------------------------------------------------------
// Public API surface
// ---------------------------------------------------------------------------

export interface IpcApi {
  getSettings(): Promise<Settings>;
  saveSettings(settings: Settings): Promise<Settings>;
  getCriteria(): Promise<Criterion[]>;
  setApiKey(key: string): Promise<void>;
  clearApiKey(): Promise<void>;
  hasApiKey(): Promise<boolean>;
  dataNotice(): Promise<{ text: string; acked: boolean }>;
  ackDataNotice(): Promise<void>;
  healthCheck(): Promise<Health>;
  startDescribe(text: string): Promise<SessionView>;
  startUpload(path: string): Promise<SessionView>;
  getSession(id: string): Promise<SessionView>;
  listSessions(): Promise<SessionSummary[]>;
  updateBrief(id: string, brief: Brief): Promise<SessionView>;
  confirmBrief(id: string): Promise<SessionView>;
  runDecisions(id: string): Promise<SessionView>;
  retry(id: string): Promise<SessionView>;
  review(args: {
    decisionId: string;
    action: ReviewAction;
    optionId?: string;
    reason?: string;
  }): Promise<DecisionView>;
  exportAdrs(id: string, dir: string): Promise<string[]>;
  exportReport(id: string, dir: string, format: "md" | "html"): Promise<string>;
  onProgress(cb: (p: Progress) => void): Promise<() => void>;
  pickDocument(): Promise<string | null>;
  pickFolder(): Promise<string | null>;
}

// ---------------------------------------------------------------------------
// Static catalogue data (mirrors catalog/criteria.yaml and catalog/decision-types.yaml)
// ---------------------------------------------------------------------------

const CRITERIA_ORDER = [
  "nfr_fit",
  "team_skill_fit",
  "cost_fit",
  "bistec_alignment",
  "lock_in",
  "security_posture",
] as const;

const CRITERIA_DEFAULTS: Criterion[] = [
  { id: "nfr_fit", name: "Fit with non-functional requirements", weight: 0.25 },
  { id: "team_skill_fit", name: "Team skill fit", weight: 0.15 },
  { id: "cost_fit", name: "Cost fit", weight: 0.2 },
  { id: "bistec_alignment", name: "BISTEC alignment", weight: 0.2 },
  { id: "lock_in", name: "Freedom from lock-in", weight: 0.1 },
  { id: "security_posture", name: "Security posture", weight: 0.1 },
];

const DATA_NOTICE_TEXT =
  "This tool sends the text of your project brief and decision requests to your " +
  "configured OpenRouter model (Jev) for architecture decisioning, and to a local " +
  "Ollama model for brief extraction. No customer data leaves your machine except " +
  "the text you explicitly submit. Every AI-proposed decision is routed to a human " +
  "reviewer before it is treated as accepted. Please confirm you understand this " +
  "before starting a session.";

const DEFAULT_SETTINGS: Settings = {
  jev_model: "typesafe/jev-1.13",
  jev_base_url: "https://openrouter.ai/api/v1",
  ollama_base_url: "http://localhost:11434",
  ollama_model: "hf.co/openbmb/MiniCPM5-2B-GGUF:Q4_K_M",
  state_token_budget: 8000,
  confidence_threshold: 0.7,
  min_margin: 0.15,
  max_questions_per_call: 3,
  max_concurrent_calls: 2,
  weights: {},
  reviewer_name: "",
};

// ---------------------------------------------------------------------------
// Internal mutable state
// ---------------------------------------------------------------------------

interface StoredSession {
  session: Session;
  sections: Section[] | null;
  docPath: "small" | "large" | null;
  brief: Brief | null;
  report: Report | null;
}

let settingsStore: Settings = clone(DEFAULT_SETTINGS);
let apiKeyValue: string | null = null;
let notice = { text: DATA_NOTICE_TEXT, acked: false };
let sessions = new Map<string, StoredSession>();
let healthOverride: Partial<Health> | null = null;
let sessionCounter = 0;
let reviewCounter = 0;
const progressListeners = new Set<(p: Progress) => void>();

function clone<T>(value: T): T {
  return JSON.parse(JSON.stringify(value));
}

function nowIso(): string {
  return new Date().toISOString();
}

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function fail(code: AppErrorShape["code"], message: string, stage?: string): never {
  throw new AppError({ code, message, stage });
}

// ---------------------------------------------------------------------------
// Brief / sections builders (insurer customer portal fixture)
// ---------------------------------------------------------------------------

function buildInsurerSections(): Section[] {
  const raw: [string, string, string][] = [
    ["S1", "Single sign-on", "Policyholders must sign in using single sign-on against the insurer's Microsoft 365 tenant, consistently across desktop and mobile browsers."],
    ["S2", "Claims intake and status", "Policyholders can submit claims and track claim status through the portal."],
    ["S3", "Policy documents", "Policyholders can retrieve and download their policy documents."],
    ["S4", "GDPR compliance", "All personal and policy data handling must be GDPR-compliant."],
    ["S5", "Billing and payment history", "Policyholders can view billing and payment history for their policies."],
    ["S6", "Budget constraints", "The project has a tight budget, favouring managed cloud services over custom infrastructure."],
    ["S7", "Engineering team", "The insurer's existing engineering team works primarily in .NET."],
    ["S8", "Scale", "The portal must scale to roughly 20,000 registered policyholders."],
    ["S9", "Audit logging", "Access to claims and policy records must be audit-logged."],
    ["S10", "Notifications", "Policyholders receive notifications when a claim's status changes."],
  ];
  return raw.map(([id, heading, text]) => ({
    id,
    heading,
    text,
    tokens: Math.round(text.split(/\s+/).length * 1.3),
  }));
}

function buildInsurerBrief(): Brief {
  return {
    summary:
      "A customer self-service portal for an insurer, serving roughly 20,000 " +
      "policyholders. Users sign in with the insurer's Microsoft 365 tenant. " +
      "The team is budget-constrained and staffed by .NET engineers. The portal " +
      "handles policy documents, claims, billing, and GDPR-regulated personal data.",
    context: {
      scale: "medium",
      budget: "tight",
      timeline: "normal",
      team_size: "small",
      compliance: ["gdpr"],
      data_sensitivity: "confidential",
    },
    requirements: [
      { text: "Support single sign-on against the insurer's Microsoft 365 tenant.", sources: ["S1"] },
      { text: "Support claims intake and status tracking for policyholders.", sources: ["S2"] },
      { text: "Support policy document retrieval and download.", sources: ["S3"] },
      { text: "Support billing and payment history for registered users.", sources: ["S5"] },
      { text: "Notify policyholders of claim status changes.", sources: ["S10"] },
    ],
    nfrs: [
      { text: "Handle personal and policy data in a GDPR-compliant manner.", sources: ["S4"] },
      { text: "Scale to roughly 20,000 registered policyholders.", sources: ["S8"] },
      { text: "Audit-log access to claims and policy records.", sources: ["S9"] },
    ],
    constraints: [
      { text: "Tight budget constraints favour managed cloud services.", sources: ["S6"] },
      { text: "The insurer's existing engineering team works primarily in .NET.", sources: ["S7"] },
    ],
    team_skills: [
      { text: ".NET / C# — the insurer's engineering team's primary stack.", sources: ["S7"] },
    ],
    mentioned_technologies: ["Microsoft 365", "Azure AD", ".NET"],
  };
}

// ---------------------------------------------------------------------------
// Report builder
// ---------------------------------------------------------------------------

interface DecisionSpec {
  typeId: string;
  typeName: string;
  options: { id: string; name: string; ring: Ring; description: string }[];
  choice: string;
  confidence: number;
  route: Route;
  reasons: ReasonCode[];
  citedSections: string[];
  /** criterion scores (0..4) per option, in CRITERIA_ORDER */
  scores: Record<string, number[]>;
  review?: { action: ReviewAction; optionId?: string; reason?: string };
}

const DECISION_SPECS: DecisionSpec[] = [
  {
    typeId: "cloud-platform",
    typeName: "Cloud platform",
    options: [
      { id: "azure", name: "Microsoft Azure", ring: "adopt", description: "BISTEC primary platform." },
      { id: "hetzner", name: "Hetzner Cloud", ring: "trial", description: "BISTEC budget platform." },
      { id: "hybrid", name: "Azure + Hetzner (hybrid)", ring: "trial", description: "Production on Azure, non-production on Hetzner." },
    ],
    choice: "azure",
    confidence: 0.93,
    route: "proposed",
    reasons: [],
    citedSections: ["S1", "S4", "S9"],
    scores: { azure: [4, 3, 2, 4, 3, 4], hetzner: [2, 1, 4, 1, 3, 2], hybrid: [3, 2, 3, 2, 2, 3] },
    review: { action: "accept" },
  },
  {
    typeId: "backend-platform",
    typeName: "Backend platform",
    options: [
      { id: "dotnet", name: ".NET 8+ / C#", ring: "adopt", description: "BISTEC primary backend." },
      { id: "node", name: "Node.js / TypeScript", ring: "trial", description: "BISTEC secondary backend." },
      { id: "java-python", name: "Java or Python", ring: "hold", description: "Avoid for web APIs unless required." },
    ],
    choice: "dotnet",
    confidence: 0.95,
    route: "proposed",
    reasons: [],
    citedSections: ["S7"],
    scores: { dotnet: [4, 4, 3, 4, 3, 3], node: [3, 1, 3, 2, 3, 3], "java-python": [2, 0, 2, 0, 2, 2] },
  },
  {
    typeId: "auth-b2c",
    typeName: "Auth (B2C)",
    options: [
      { id: "azure-ad-b2c", name: "Azure AD B2C", ring: "adopt", description: "Consumer identity on Azure." },
      { id: "auth0", name: "Auth0", ring: "trial", description: "Third-party identity platform." },
      { id: "firebase-auth", name: "Firebase Auth", ring: "hold", description: "Google identity service; avoid." },
    ],
    choice: "azure-ad-b2c",
    confidence: 0.61,
    route: "needs_architect",
    reasons: ["close_margin"],
    citedSections: ["S1"],
    scores: { "azure-ad-b2c": [3, 2, 2, 4, 3, 4], auth0: [3, 2, 2, 2, 2, 3], "firebase-auth": [2, 1, 3, 0, 1, 2] },
  },
  {
    typeId: "relational-db-enterprise",
    typeName: "Relational DB (enterprise)",
    options: [
      { id: "sql-server", name: "SQL Server", ring: "adopt", description: "Primary RDBMS for transactional data." },
      { id: "postgresql", name: "PostgreSQL", ring: "trial", description: "Open-source RDBMS." },
      { id: "mysql", name: "MySQL", ring: "hold", description: "Avoid." },
      { id: "sqlite-prod", name: "SQLite (prod)", ring: "hold", description: "Avoid for production enterprise data." },
    ],
    choice: "sql-server",
    confidence: 0.88,
    route: "proposed",
    reasons: [],
    citedSections: ["S6", "S9"],
    scores: {
      "sql-server": [4, 2, 2, 4, 2, 4],
      postgresql: [3, 1, 4, 2, 3, 3],
      mysql: [2, 1, 3, 0, 3, 2],
      "sqlite-prod": [1, 1, 4, 0, 4, 1],
    },
    review: {
      action: "override",
      optionId: "postgresql",
      reason: "The insurer's ops team already runs PostgreSQL on Azure; overriding avoids new licensing cost given the tight budget.",
    },
  },
  {
    typeId: "document-db",
    typeName: "Document DB",
    options: [
      { id: "cosmos-db", name: "Cosmos DB (SQL API)", ring: "adopt", description: "Primary NoSQL." },
      { id: "mongodb-hetzner", name: "MongoDB on Hetzner", ring: "trial", description: "Self-hosted document database." },
      { id: "dynamodb", name: "DynamoDB", ring: "hold", description: "AWS-only; avoid." },
    ],
    choice: "mongodb-hetzner",
    confidence: 0.55,
    route: "needs_architect",
    reasons: ["disagreement", "low_confidence"],
    citedSections: ["S6"],
    scores: { "cosmos-db": [4, 1, 2, 4, 2, 4], "mongodb-hetzner": [2, 1, 4, 1, 3, 2], dynamodb: [3, 0, 2, 0, 1, 3] },
  },
  {
    typeId: "background-jobs",
    typeName: "Background jobs",
    options: [
      { id: "azure-functions", name: "Azure Functions", ring: "adopt", description: "Isolated Worker on .NET 8+." },
      { id: "hangfire", name: "Hangfire", ring: "adopt", description: ".NET background job processing in-process." },
      { id: "bullmq", name: "BullMQ (Node)", ring: "trial", description: "Redis-backed job queue for Node." },
      { id: "cron-scripts", name: "cron + scripts", ring: "hold", description: "Ad-hoc scheduled scripts; avoid." },
    ],
    choice: "cron-scripts",
    confidence: 0.42,
    route: "needs_architect",
    reasons: ["hold_option", "low_confidence"],
    citedSections: ["S9"],
    scores: {
      "azure-functions": [4, 3, 3, 4, 3, 3],
      hangfire: [3, 4, 3, 3, 3, 3],
      bullmq: [3, 1, 3, 2, 3, 3],
      "cron-scripts": [1, 2, 4, 0, 1, 0],
    },
    review: {
      action: "reject",
      reason: "cron + scripts cannot meet the audit-logging NFR; use Azure Functions instead.",
    },
  },
  {
    typeId: "frontend",
    typeName: "Frontend",
    options: [
      { id: "nextjs-react", name: "Next.js + React", ring: "adopt", description: "React 18+ / Next.js 14+ with Tailwind." },
      { id: "blazor", name: "Blazor (.NET teams)", ring: "trial", description: "For .NET-heavy teams." },
      { id: "angular", name: "Angular (new projects)", ring: "hold", description: "Avoid for new projects." },
    ],
    choice: "nextjs-react",
    confidence: 0.9,
    route: "proposed",
    reasons: [],
    citedSections: ["S7"],
    scores: { "nextjs-react": [3, 2, 3, 4, 3, 3], blazor: [3, 4, 3, 2, 3, 3], angular: [2, 1, 3, 0, 2, 2] },
    review: { action: "accept" },
  },
  {
    typeId: "css",
    typeName: "CSS",
    options: [
      { id: "tailwind", name: "Tailwind CSS", ring: "adopt", description: "Utility-first CSS." },
      { id: "css-modules", name: "CSS Modules", ring: "trial", description: "Locally scoped CSS files." },
    ],
    choice: "tailwind",
    confidence: 0.97,
    route: "proposed",
    reasons: [],
    citedSections: [],
    scores: { tailwind: [3, 2, 4, 4, 4, 3], "css-modules": [3, 3, 4, 1, 4, 3] },
  },
  {
    typeId: "full-text-search",
    typeName: "Full-text search",
    options: [
      { id: "azure-ai-search", name: "Azure AI Search", ring: "adopt", description: "Full-text, vector and semantic ranking search." },
      { id: "elasticsearch-hetzner", name: "Elasticsearch (Hetzner)", ring: "trial", description: "Self-hosted search." },
      { id: "sql-like", name: "SQL LIKE queries", ring: "hold", description: "Pattern matching in SQL; avoid." },
    ],
    choice: "azure-ai-search",
    confidence: 0.7,
    route: "needs_architect",
    reasons: ["possible_injection"],
    citedSections: ["S2"],
    scores: {
      "azure-ai-search": [4, 2, 2, 4, 2, 4],
      "elasticsearch-hetzner": [3, 1, 4, 1, 3, 2],
      "sql-like": [1, 3, 4, 0, 4, 1],
    },
  },
];

function weightedComposite(scores: number[]): number {
  const weights = CRITERIA_DEFAULTS.map((c) => c.weight);
  let sum = 0;
  for (let i = 0; i < scores.length; i++) {
    sum += (scores[i] / 4) * weights[i];
  }
  return Math.round(sum * 1000) / 1000;
}

function buildDecisionView(spec: DecisionSpec, sessionId: string, index: number): DecisionView {
  const decisionId = `${sessionId}-dec-${index}`;
  const optionIds = spec.options.map((o) => o.id);
  const winnerShare = spec.confidence;
  const remainder = optionIds.length > 1 ? (1 - winnerShare) / (optionIds.length - 1) : 0;
  const probabilities: Record<string, number> = {};
  for (const id of optionIds) {
    probabilities[id] = id === spec.choice ? winnerShare : Math.round(remainder * 1000) / 1000;
  }

  const optionScores = spec.options.map((option) => {
    const raw = spec.scores[option.id];
    const criterionScores: Record<string, number> = {};
    CRITERIA_ORDER.forEach((cid, i) => {
      criterionScores[cid] = raw[i];
    });
    return {
      option_id: option.id,
      criterion_scores: criterionScores,
      composite: weightedComposite(raw),
    };
  });

  const decision = {
    id: decisionId,
    session_id: sessionId,
    type_id: spec.typeId,
    choice: spec.choice,
    confidence: spec.confidence,
    probabilities,
    option_scores: optionScores,
    route: spec.route,
    reasons: spec.reasons,
    model_snapshot: "typesafe/jev-1.13",
    request_hash: `mockhash-${sessionId}-${index}`,
    cited_sections: spec.citedSections,
  };

  const reviews: Review[] = [];
  if (spec.review) {
    reviews.push({
      id: ++reviewCounter,
      decision_id: decisionId,
      action: spec.review.action,
      option_id: spec.review.optionId ?? null,
      reviewer: "A. Fernando",
      reason: spec.review.reason ?? null,
      at_utc: nowIso(),
    });
  }

  const { status, label } = deriveStatus(reviews);

  return {
    decision,
    type_name: spec.typeName,
    options: spec.options,
    reviews,
    status,
    status_label: label,
  };
}

function deriveStatus(reviews: Review[]): { status: DecisionView["status"]; label: string } {
  if (reviews.length === 0) {
    return { status: "proposed_ai", label: "Proposed (AI) — pending approval" };
  }
  const last = reviews[reviews.length - 1];
  switch (last.action) {
    case "accept":
      return { status: "accepted", label: "Accepted" };
    case "override":
      return { status: "accepted_override", label: "Accepted (override)" };
    case "reject":
      return { status: "rejected", label: "Rejected" };
  }
}

function buildInsurerReport(sessionId: string): Report {
  const decisions = DECISION_SPECS.map((spec, i) => buildDecisionView(spec, sessionId, i + 1));
  return {
    gates: { is_technical_request: 0.97, has_enough_context: 0.91, injection: 0.04 },
    decisions,
    not_applicable: [
      { type_id: "real-time", type_name: "Real-time", probability: 0.06 },
      { type_id: "message-queue-simple", type_name: "Message queue (simple)", probability: 0.11 },
      { type_id: "auth-enterprise", type_name: "Auth (enterprise)", probability: 0.08 },
    ],
    input_tokens: 15234,
    cost_usd: 0.0842,
    truncated: false,
    criteria: clone(CRITERIA_DEFAULTS),
  };
}

// ---------------------------------------------------------------------------
// Session helpers
// ---------------------------------------------------------------------------

function deriveTitle(text: string): string {
  const words = text.trim().split(/\s+/);
  const short = words.slice(0, 8).join(" ");
  return words.length > 8 ? `${short}…` : short || "Untitled session";
}

function toSessionView(stored: StoredSession): SessionView {
  return clone({
    session: stored.session,
    sections: stored.sections,
    doc_path: stored.docPath,
    brief: stored.brief,
    report: stored.report,
  });
}

function getStoredOrThrow(id: string): StoredSession {
  const stored = sessions.get(id);
  if (!stored) fail("not_found", `No session found with id "${id}".`);
  return stored;
}

function toSummary(stored: StoredSession): SessionSummary {
  const decisions = stored.report?.decisions ?? [];
  return {
    id: stored.session.id,
    created_at: stored.session.created_at,
    title: stored.session.title,
    mode: stored.session.mode,
    stage: stored.session.stage,
    decision_count: decisions.length,
    needs_architect_count: decisions.filter((d) => d.decision.route === "needs_architect").length,
    unreviewed_count: decisions.filter((d) => d.reviews.length === 0).length,
  };
}

function createSession(
  mode: SessionMode,
  opts: { inputText?: string; docName?: string; id?: string },
): StoredSession {
  const id = opts.id ?? `session-${++sessionCounter}`;
  const title = mode === "describe" ? deriveTitle(opts.inputText ?? "") : opts.docName ?? "Uploaded document";
  const session: Session = {
    id,
    created_at: nowIso(),
    title,
    mode,
    input_text: opts.inputText ?? null,
    doc_name: opts.docName ?? null,
    stage: "review",
    error: null,
    brief_confirmed_at: null,
  };
  const stored: StoredSession = {
    session,
    sections: buildInsurerSections(),
    docPath: mode === "upload" ? "small" : null,
    brief: buildInsurerBrief(),
    report: null,
  };
  sessions.set(id, stored);
  return stored;
}

function emitProgress(p: Progress) {
  for (const cb of progressListeners) cb(p);
}

async function runDecisionsInternal(id: string): Promise<SessionView> {
  const stored = getStoredOrThrow(id);
  if (!stored.session.brief_confirmed_at) {
    fail("validation", "Confirm the brief before running decisions.");
  }
  if (!apiKeyValue) {
    fail("no_api_key", "Set an OpenRouter API key in Settings before running decisions.");
  }
  if (!notice.acked) {
    fail("data_notice_required", "Acknowledge the data notice before running decisions.");
  }

  const stages: { stage: string; total: number }[] = [
    { stage: "gates", total: 3 },
    { stage: "platforms", total: 2 },
    { stage: "applicability", total: 25 },
    { stage: "decisions", total: DECISION_SPECS.length },
  ];

  for (const s of stages) {
    stored.session.stage = s.stage;
    for (let done = 1; done <= s.total; done++) {
      await delay(15);
      emitProgress({ session_id: id, stage: s.stage, done, total: s.total, message: null });
    }
  }

  stored.report = buildInsurerReport(id);
  stored.session.stage = "done";
  return toSessionView(stored);
}

// ---------------------------------------------------------------------------
// Seed data
// ---------------------------------------------------------------------------

function seed() {
  const done = createSession("describe", {
    id: "session-seed-report",
    inputText:
      "Build a customer self-service portal for our insurance clients, roughly 20,000 " +
      "policyholders, signing in via our Microsoft 365 tenant. Budget is tight and our " +
      "team is all .NET. Needs to handle claims, policy documents, billing, and GDPR data.",
  });
  done.session.brief_confirmed_at = nowIso();
  done.session.stage = "done";
  done.report = buildInsurerReport(done.session.id);

  const inReview = createSession("upload", {
    id: "session-seed-brief",
    docName: "customer-portal-requirements.docx",
  });
  inReview.docPath = "large";
}

export function resetMock() {
  settingsStore = clone(DEFAULT_SETTINGS);
  apiKeyValue = null;
  notice = { text: DATA_NOTICE_TEXT, acked: false };
  sessions = new Map();
  healthOverride = null;
  sessionCounter = 0;
  reviewCounter = 0;
  progressListeners.clear();
  seed();
}

/** Test-only hook: force the next health_check() result. Pass null to clear. */
export function setHealthOverride(overrides: Partial<Health> | null) {
  healthOverride = overrides;
}

seed();

// ---------------------------------------------------------------------------
// mockApi
// ---------------------------------------------------------------------------

function effectiveWeights(raw: Record<string, number>): Record<string, number> {
  const merged: Record<string, number> = {};
  for (const c of CRITERIA_DEFAULTS) merged[c.id] = raw[c.id] ?? c.weight;
  return merged;
}

function effectiveSettings(): Settings {
  return clone({ ...settingsStore, weights: effectiveWeights(settingsStore.weights) });
}

export const mockApi: IpcApi = {
  async getSettings() {
    return effectiveSettings();
  },

  async saveSettings(settings: Settings) {
    settingsStore = clone(settings);
    return effectiveSettings();
  },

  async getCriteria() {
    return clone(CRITERIA_DEFAULTS);
  },

  async setApiKey(key: string) {
    if (!key.trim()) fail("validation", "API key cannot be blank.");
    apiKeyValue = key;
  },

  async clearApiKey() {
    apiKeyValue = null;
  },

  async hasApiKey() {
    return apiKeyValue !== null && apiKeyValue.length > 0;
  },

  async dataNotice() {
    return { ...notice };
  },

  async ackDataNotice() {
    notice = { ...notice, acked: true };
  },

  async healthCheck() {
    const base: Health = {
      ollama_reachable: true,
      model_present: true,
      model: settingsStore.ollama_model,
      pull_command: `ollama pull ${settingsStore.ollama_model}`,
      openrouter: apiKeyValue ? "ok" : "no_key",
      openrouter_error: null,
    };
    return { ...base, ...(healthOverride ?? {}) };
  },

  async startDescribe(text: string) {
    const stored = createSession("describe", { inputText: text });
    return toSessionView(stored);
  },

  async startUpload(path: string) {
    const docName = path.split(/[\\/]/).pop() ?? path;
    const stored = createSession("upload", { docName });
    return toSessionView(stored);
  },

  async getSession(id: string) {
    return toSessionView(getStoredOrThrow(id));
  },

  async listSessions() {
    return [...sessions.values()]
      .slice()
      .sort((a, b) => (a.session.created_at < b.session.created_at ? 1 : -1))
      .map(toSummary);
  },

  async updateBrief(id: string, brief: Brief) {
    const stored = getStoredOrThrow(id);
    stored.brief = clone(brief);
    return toSessionView(stored);
  },

  async confirmBrief(id: string) {
    const stored = getStoredOrThrow(id);
    if (!stored.brief) fail("validation", "There is no brief to confirm yet.");
    stored.session.brief_confirmed_at = nowIso();
    return toSessionView(stored);
  },

  async runDecisions(id: string) {
    return runDecisionsInternal(id);
  },

  async retry(id: string) {
    const stored = getStoredOrThrow(id);
    if (stored.session.stage.startsWith("failed:") || stored.session.stage === "out_of_remit") {
      return runDecisionsInternal(id);
    }
    return toSessionView(stored);
  },

  async review({ decisionId, action, optionId, reason }) {
    const reviewer = settingsStore.reviewer_name.trim();
    if (!reviewer) {
      fail("validation", "Reviewer name is required. Set it in Settings before reviewing decisions.");
    }
    if (action === "override" && !optionId) {
      fail("validation", "Select an option to override to.");
    }
    if ((action === "override" || action === "reject") && !reason?.trim()) {
      fail("validation", "A reason is required to override or reject a decision.");
    }

    for (const stored of sessions.values()) {
      if (!stored.report) continue;
      const view = stored.report.decisions.find((d) => d.decision.id === decisionId);
      if (!view) continue;
      if (action === "override" && !view.options.some((o) => o.id === optionId)) {
        fail("validation", "The selected option does not belong to this decision.");
      }
      view.reviews.push({
        id: ++reviewCounter,
        decision_id: decisionId,
        action,
        option_id: action === "override" ? optionId! : null,
        reviewer,
        reason: reason?.trim() || null,
        at_utc: nowIso(),
      });
      const { status, label } = deriveStatus(view.reviews);
      view.status = status;
      view.status_label = label;
      return clone(view);
    }
    fail("not_found", `No decision found with id "${decisionId}".`);
  },

  async exportAdrs(id: string, dir: string) {
    const stored = getStoredOrThrow(id);
    const decisions = stored.report?.decisions ?? [];
    return decisions.map(
      (d, i) => `${dir}/ADR-${String(i + 1).padStart(4, "0")}-${d.decision.type_id}.md`,
    );
  },

  async exportReport(id: string, dir: string, format: "md" | "html") {
    getStoredOrThrow(id);
    return `${dir}/report.${format}`;
  },

  async onProgress(cb: (p: Progress) => void) {
    progressListeners.add(cb);
    return () => {
      progressListeners.delete(cb);
    };
  },

  async pickDocument() {
    return "/mock/inbox/customer-portal-requirements.docx";
  },

  async pickFolder() {
    return "/mock/exports";
  },
};
