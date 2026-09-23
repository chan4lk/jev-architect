import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";

import { ScoreHeatmap } from "./ScoreHeatmap";
import type { Criterion, OptionScore, OptionView } from "@/lib/schemas";

const criteria: Criterion[] = [
  { id: "nfr_fit", name: "NFR fit", weight: 0.5 },
  { id: "cost_fit", name: "Cost fit", weight: 0.5 },
];

const options: OptionView[] = [
  { id: "adopt-opt", name: "SQL Server", ring: "adopt", description: "" },
  { id: "hold-opt", name: "MySQL", ring: "hold", description: "" },
];

const optionScores: OptionScore[] = [
  { option_id: "adopt-opt", criterion_scores: { nfr_fit: 4, cost_fit: 2 }, composite: 0.75 },
  // "hold-opt" intentionally has no matching entry — it was never asked Score questions.
];

describe("ScoreHeatmap", () => {
  it("shows the composite as a rounded percentage", () => {
    render(<ScoreHeatmap options={options} optionScores={optionScores} criteria={criteria} />);
    expect(screen.getByText("75%")).toBeInTheDocument();
    expect(screen.getByText("4")).toBeInTheDocument();
    expect(screen.getByText("2")).toBeInTheDocument();
  });

  it("renders '—' in every cell for an unscored hold option", () => {
    render(<ScoreHeatmap options={options} optionScores={optionScores} criteria={criteria} />);
    // 2 criterion cells + 1 composite cell for the unscored option.
    expect(screen.getAllByText("—")).toHaveLength(3);
  });
});
