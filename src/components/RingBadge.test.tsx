import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";

import { RingBadge } from "./RingBadge";

describe("RingBadge", () => {
  it.each([
    ["adopt", "Adopt"],
    ["trial", "Trial"],
    ["hold", "Hold"],
  ] as const)("always renders a text label for ring %s, not colour alone", (ring, label) => {
    render(<RingBadge ring={ring} />);
    expect(screen.getByText(label)).toBeInTheDocument();
  });
});
