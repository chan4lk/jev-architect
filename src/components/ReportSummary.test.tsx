import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";

import { ReportSummary } from "./ReportSummary";
import type { Report } from "@/lib/schemas";

function makeReport(overrides: Partial<Report> = {}): Report {
  return {
    gates: { is_technical_request: 0.9, has_enough_context: 0.9, injection: 0.02 },
    decisions: [],
    not_applicable: [],
    input_tokens: 1234,
    cost_usd: 0.0123,
    truncated: false,
    criteria: [{ id: "nfr_fit", name: "NFR fit", weight: 1 }],
    ...overrides,
  };
}

describe("ReportSummary", () => {
  it("formats a known cost to 4 decimal places", () => {
    render(<ReportSummary report={makeReport({ cost_usd: 0.0123 })} brief={null} />);
    expect(screen.getByText("$0.0123")).toBeInTheDocument();
  });

  it('shows "n/a" when cost_usd is null', () => {
    render(<ReportSummary report={makeReport({ cost_usd: null })} brief={null} />);
    expect(screen.getByText("n/a")).toBeInTheDocument();
  });

  it("shows the thin-context, truncated, and injection banners when gates cross their thresholds", () => {
    render(
      <ReportSummary
        report={makeReport({
          gates: { is_technical_request: 0.9, has_enough_context: 0.3, injection: 0.4 },
          truncated: true,
        })}
        brief={null}
      />,
    );
    expect(screen.getByText(/decisions may be unreliable/)).toBeInTheDocument();
    expect(screen.getByText(/dropped to fit the token budget/)).toBeInTheDocument();
    expect(screen.getByText(/every decision needs an architect/)).toBeInTheDocument();
  });

  it("does not show gate banners when nothing crosses a threshold", () => {
    render(<ReportSummary report={makeReport()} brief={null} />);
    expect(screen.queryByText(/decisions may be unreliable/)).not.toBeInTheDocument();
    expect(screen.queryByText(/dropped to fit the token budget/)).not.toBeInTheDocument();
    expect(screen.queryByText(/every decision needs an architect/)).not.toBeInTheDocument();
  });

  it("labels the criterion weights as uncalibrated defaults", () => {
    render(<ReportSummary report={makeReport()} brief={null} />);
    expect(screen.getByText("Weights and thresholds are uncalibrated defaults")).toBeInTheDocument();
  });
});
