import { beforeEach, describe, expect, it } from "vitest";
import { mockApi, resetMock } from "./ipc-mock";
import {
  CriterionSchema,
  HealthSchema,
  ReportSchema,
  SessionSummarySchema,
  SessionViewSchema,
  SettingsSchema,
} from "./schemas";
import { z } from "zod";

describe("schemas parse the mock IPC data", () => {
  beforeEach(() => resetMock());

  it("parses get_settings", async () => {
    const settings = await mockApi.getSettings();
    expect(() => SettingsSchema.parse(settings)).not.toThrow();
  });

  it("parses get_criteria", async () => {
    const criteria = await mockApi.getCriteria();
    expect(() => z.array(CriterionSchema).parse(criteria)).not.toThrow();
  });

  it("parses health_check", async () => {
    const health = await mockApi.healthCheck();
    expect(() => HealthSchema.parse(health)).not.toThrow();
  });

  it("parses list_sessions and every get_session result", async () => {
    const summaries = await mockApi.listSessions();
    expect(summaries.length).toBeGreaterThan(0);
    for (const summary of summaries) {
      expect(() => SessionSummarySchema.parse(summary)).not.toThrow();
      const view = SessionViewSchema.parse(await mockApi.getSession(summary.id));
      if (view.report) {
        ReportSchema.parse(view.report);
      }
    }
  });

  it("the seeded completed session has a thorough report", async () => {
    const view = await mockApi.getSession("session-seed-report");
    const report = ReportSchema.parse(view.report);

    expect(report.decisions.length).toBeGreaterThanOrEqual(6);
    const routes = new Set(report.decisions.map((d) => d.decision.route));
    expect(routes.has("proposed")).toBe(true);
    expect(routes.has("needs_architect")).toBe(true);

    const rings = new Set(
      report.decisions.flatMap((d) => d.options.filter((o) => o.id === d.decision.choice).map((o) => o.ring)),
    );
    expect(rings.has("adopt")).toBe(true);
    expect(rings.has("trial")).toBe(true);
    expect(rings.has("hold")).toBe(true);

    const reasonCodes = new Set(report.decisions.flatMap((d) => d.decision.reasons));
    expect(reasonCodes.size).toBeGreaterThanOrEqual(3);

    expect(report.not_applicable.length).toBeGreaterThan(0);
    expect(report.criteria.length).toBe(6);
  });
});
