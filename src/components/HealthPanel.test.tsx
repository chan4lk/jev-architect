import { beforeEach, describe, expect, it } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import { HealthPanel } from "./HealthPanel";
import { resetMock, setHealthOverride } from "@/lib/ipc-mock";

function renderHealthPanel() {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={queryClient}>
      <HealthPanel />
    </QueryClientProvider>,
  );
}

describe("HealthPanel", () => {
  beforeEach(() => {
    resetMock();
    setHealthOverride(null);
  });

  it("shows the exact pull command when the model is missing", async () => {
    const pullCommand = "ollama pull hf.co/openbmb/MiniCPM5-2B-GGUF:Q4_K_M";
    setHealthOverride({
      model_present: false,
      model: "hf.co/openbmb/MiniCPM5-2B-GGUF:Q4_K_M",
      pull_command: pullCommand,
    });

    renderHealthPanel();
    fireEvent.click(screen.getByRole("button", { name: /check connections/i }));

    expect(await screen.findByText(pullCommand)).toBeInTheDocument();
  });

  it("does not show a pull command when the model is present", async () => {
    renderHealthPanel();
    fireEvent.click(screen.getByRole("button", { name: /check connections/i }));

    await screen.findByText("Ollama reachable");
    expect(screen.queryByText(/ollama pull/i)).not.toBeInTheDocument();
  });
});
