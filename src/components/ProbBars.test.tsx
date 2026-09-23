import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";

import { ProbBars } from "./ProbBars";
import type { OptionView } from "@/lib/schemas";

const options: OptionView[] = [
  { id: "a", name: "Azure", ring: "adopt", description: "" },
  { id: "b", name: "Hetzner", ring: "trial", description: "" },
  { id: "c", name: "Hybrid", ring: "trial", description: "" },
];

describe("ProbBars", () => {
  it("sorts bars by probability, descending, and labels each with its percentage", () => {
    render(<ProbBars options={options} probabilities={{ a: 0.6, b: 0.1, c: 0.3 }} choice="a" />);

    const bars = screen.getAllByRole("img");
    expect(bars.map((bar) => bar.getAttribute("aria-label"))).toEqual([
      "Azure: 60%",
      "Hybrid: 30%",
      "Hetzner: 10%",
    ]);
    expect(screen.getByText("60%")).toBeInTheDocument();
    expect(screen.getByText("30%")).toBeInTheDocument();
    expect(screen.getByText("10%")).toBeInTheDocument();
  });

  it("emphasises the chosen option's name", () => {
    render(<ProbBars options={options} probabilities={{ a: 0.6, b: 0.1, c: 0.3 }} choice="a" />);
    expect(screen.getByText("Azure")).toHaveClass("font-semibold");
    expect(screen.getByText("Hetzner")).not.toHaveClass("font-semibold");
  });
});
