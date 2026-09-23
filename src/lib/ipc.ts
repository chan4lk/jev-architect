/**
 * The single entry point the rest of the app uses to talk to the backend.
 *
 * This is the ONLY file that imports `@tauri-apps/*`. At startup it picks a
 * concrete implementation of `IpcApi`:
 *  - inside a Tauri window (`window.__TAURI_INTERNALS__` present): the real
 *    `invoke`/`listen`/dialog-backed implementation below.
 *  - otherwise (plain browser, Vitest, Playwright e2e): `mockApi` from
 *    `./ipc-mock`.
 *
 * Every command result is parsed with its Zod schema from `./schemas` so a
 * shape mismatch between the Rust DTOs and this file fails loudly instead of
 * producing `undefined` deep inside a component. Every rejection is
 * normalised to a thrown `AppError`.
 */
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { z } from "zod";

import {
  AppError,
  AppErrorSchema,
  CriterionSchema,
  DataNoticeSchema,
  DecisionViewSchema,
  HealthSchema,
  LocalModelStatusSchema,
  ProgressSchema,
  SessionSummarySchema,
  SessionViewSchema,
  SettingsSchema,
  type Brief,
  type Progress,
  type Settings,
} from "./schemas";
import { mockApi, type IpcApi } from "./ipc-mock";

export { AppError } from "./schemas";
export type { IpcApi } from "./ipc-mock";

function toAppError(err: unknown): AppError {
  if (err instanceof AppError) return err;
  const parsed = AppErrorSchema.safeParse(err);
  if (parsed.success) return new AppError(parsed.data);
  const message = typeof err === "string" ? err : err instanceof Error ? err.message : "Something went wrong.";
  return new AppError({ code: "internal", message });
}

async function call<T>(
  cmd: string,
  args: Record<string, unknown> | undefined,
  schema: z.ZodType<T>,
): Promise<T> {
  let raw: unknown;
  try {
    raw = await invoke(cmd, args);
  } catch (e) {
    throw toAppError(e);
  }
  const parsed = schema.safeParse(raw);
  if (!parsed.success) {
    throw new AppError({
      code: "internal",
      message: `Unexpected response shape from "${cmd}": ${parsed.error.message}`,
    });
  }
  return parsed.data;
}

async function callVoid(cmd: string, args?: Record<string, unknown>): Promise<void> {
  try {
    await invoke(cmd, args);
  } catch (e) {
    throw toAppError(e);
  }
}

const tauriApi: IpcApi = {
  getSettings: () => call("get_settings", undefined, SettingsSchema),
  saveSettings: (settings: Settings) => call("save_settings", { settings }, SettingsSchema),
  getCriteria: () => call("get_criteria", undefined, z.array(CriterionSchema)),
  setApiKey: (key: string) => callVoid("set_api_key", { key }),
  clearApiKey: () => callVoid("clear_api_key"),
  hasApiKey: () => call("has_api_key", undefined, z.boolean()),
  dataNotice: () => call("data_notice", undefined, DataNoticeSchema),
  ackDataNotice: () => callVoid("ack_data_notice"),
  healthCheck: () => call("health_check", undefined, HealthSchema),
  localModelStatus: () => call("local_model_status", undefined, LocalModelStatusSchema),
  startDescribe: (text: string) => call("start_describe", { text }, SessionViewSchema),
  startUpload: (path: string) => call("start_upload", { path }, SessionViewSchema),
  getSession: (id: string) => call("get_session", { id }, SessionViewSchema),
  listSessions: () => call("list_sessions", undefined, z.array(SessionSummarySchema)),
  updateBrief: (id: string, brief: Brief) => call("update_brief", { id, brief }, SessionViewSchema),
  confirmBrief: (id: string) => call("confirm_brief", { id }, SessionViewSchema),
  runDecisions: (id: string) => call("run_decisions", { id }, SessionViewSchema),
  retry: (id: string) => call("retry", { id }, SessionViewSchema),
  review: (args) => call("review", args, DecisionViewSchema),
  exportAdrs: (id: string, dir: string) => call("export_adrs", { id, dir }, z.array(z.string())),
  exportReport: (id: string, dir: string, format: "md" | "html") =>
    call("export_report", { id, dir, format }, z.string()),

  onProgress: async (cb: (p: Progress) => void) => {
    return listen<unknown>("pipeline://progress", (event) => {
      const parsed = ProgressSchema.safeParse(event.payload);
      if (parsed.success) cb(parsed.data);
    });
  },

  pickDocument: async () => {
    const result = await open({
      multiple: false,
      filters: [{ name: "Requirements documents", extensions: ["pdf", "docx", "md", "txt"] }],
    });
    return typeof result === "string" ? result : null;
  },

  pickFolder: async () => {
    const result = await open({ directory: true, multiple: false });
    return typeof result === "string" ? result : null;
  },
};

const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

/** The IPC surface used by every screen/component. Swapped for `mockApi` outside Tauri. */
export const api: IpcApi = isTauri ? tauriApi : mockApi;

/**
 * Human-readable, health-check-referencing copy for the two error codes that
 * most commonly hit users on the Home screen (AC-related copy requirement).
 */
export function describeStartError(err: AppError): string {
  switch (err.code) {
    case "brief_extraction_failed":
      return `Could not extract a brief from that input (${err.message}). Run the health check in Settings to confirm the local model is reachable.`;
    case "local_model":
      return `The local model is unavailable (${err.message}). Run the health check in Settings to confirm Ollama is reachable and the model is pulled.`;
    case "no_api_key":
      return "Set an OpenRouter API key in Settings before starting a session.";
    case "data_notice_required":
      return "Acknowledge the data notice before starting a session.";
    default:
      return err.message;
  }
}
