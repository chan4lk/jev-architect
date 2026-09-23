/// <reference types="node" />
/**
 * Cross-language IPC contract check: every fixture here is the real
 * serialized output of a Rust command, written by
 * `src-tauri/tests/commands.rs` (`cargo test`). Each must parse with the
 * Zod schema the UI applies to that command's result.
 */
import { readFileSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { z } from "zod";

import {
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
} from "./schemas";

const FIXTURES = path.resolve(process.cwd(), "src-tauri/tests/fixtures/contract");

const cases: [string, z.ZodType][] = [
  ["settings.json", SettingsSchema],
  ["criteria.json", z.array(CriterionSchema)],
  ["session_view_review.json", SessionViewSchema],
  ["session_view_done.json", SessionViewSchema],
  ["session_summaries.json", z.array(SessionSummarySchema)],
  ["decision_view_reviewed.json", DecisionViewSchema],
  ["health.json", HealthSchema],
  ["local_model_status.json", LocalModelStatusSchema],
  ["data_notice.json", DataNoticeSchema],
  ["progress_event.json", ProgressSchema],
  ["app_error.json", AppErrorSchema],
];

function load(name: string): unknown {
  return JSON.parse(readFileSync(path.join(FIXTURES, name), "utf8"));
}

describe("Rust command output matches the UI's Zod schemas", () => {
  it.each(cases)("%s", (name, schema) => {
    const result = schema.safeParse(load(name));
    expect(result.success, result.success ? "" : result.error.message).toBe(true);
  });

  it("the review fixture is at stage review and the done fixture carries a report", () => {
    const review = SessionViewSchema.parse(load("session_view_review.json"));
    expect(review.session.stage).toBe("review");
    expect(review.brief).not.toBeNull();
    const done = SessionViewSchema.parse(load("session_view_done.json"));
    expect(done.session.stage).toBe("done");
    expect(done.report?.decisions.length).toBeGreaterThan(0);
  });

  it("the reviewed decision carries its append-only history", () => {
    const view = DecisionViewSchema.parse(load("decision_view_reviewed.json"));
    expect(view.reviews.length).toBe(2);
    expect(view.status).toBe("accepted_override");
  });
});
