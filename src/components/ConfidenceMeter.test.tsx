import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";

import { ConfidenceMeter } from "./ConfidenceMeter";

describe("ConfidenceMeter", () => {
  it("exposes the confidence percentage and threshold via ARIA and text", () => {
    render(<ConfidenceMeter confidence={0.72} threshold={0.5} />);
    const meter = screen.getByRole("meter");
    expect(meter).toHaveAttribute("aria-valuenow", "72");
    expect(meter).toHaveAttribute("aria-valuetext", "72% confidence, threshold 50%");
    expect(screen.getByText(/72% \(threshold 50%\)/)).toBeInTheDocument();
  });
});
