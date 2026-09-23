import { beforeEach, describe, expect, it } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import { Home } from "./Home";
import { resetMock, setLocalModelStatusOverride } from "@/lib/ipc-mock";

function renderHome() {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={queryClient}>
      <MemoryRouter initialEntries={["/"]}>
        <Routes>
          <Route path="/" element={<Home />} />
          <Route path="/session/:id/brief" element={<div>Brief screen</div>} />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

describe("Home — Describe tab", () => {
  beforeEach(() => resetMock());

  it("disables the submit button under 20 characters and enables it at 20+", async () => {
    renderHome();
    const textarea = await screen.findByLabelText("Project description");
    const button = screen.getByRole("button", { name: /extract brief/i });
    expect(button).toBeDisabled();

    fireEvent.change(textarea, { target: { value: "too short" } });
    expect(button).toBeDisabled();

    fireEvent.change(textarea, { target: { value: "a".repeat(20) } });
    expect(button).not.toBeDisabled();
  });

  it("navigates to the brief route after submitting, once the data notice is acknowledged", async () => {
    renderHome();
    const textarea = await screen.findByLabelText("Project description");
    fireEvent.change(textarea, { target: { value: "a".repeat(30) } });
    fireEvent.click(screen.getByRole("button", { name: /extract brief/i }));

    const ackButton = await screen.findByRole("button", { name: /acknowledge & continue/i });
    fireEvent.click(ackButton);

    expect(await screen.findByText("Brief screen")).toBeInTheDocument();
  });
});

describe("Home — data notice gate", () => {
  beforeEach(() => resetMock());

  it("blocks starting a session until the notice is acknowledged", async () => {
    renderHome();
    const textarea = await screen.findByLabelText("Project description");
    fireEvent.change(textarea, { target: { value: "a".repeat(30) } });
    fireEvent.click(screen.getByRole("button", { name: /extract brief/i }));

    expect(await screen.findByText("Data notice")).toBeInTheDocument();
    expect(screen.queryByText("Brief screen")).not.toBeInTheDocument();
  });
});

describe("Home — local model gating (edge case: Ollama down / model not pulled)", () => {
  beforeEach(() => resetMock());

  it("disables Describe and shows the pull command when the model isn't pulled, but Upload stays enabled", async () => {
    setLocalModelStatusOverride({ ollama_reachable: true, model_present: false });
    renderHome();

    expect(await screen.findByText("Local model unavailable")).toBeInTheDocument();
    expect(await screen.findByText("ollama pull hf.co/openbmb/MiniCPM5-2B-GGUF:Q4_K_M")).toBeInTheDocument();
    expect(screen.getByText(/still works/i)).toBeInTheDocument();

    const textarea = await screen.findByLabelText("Project description");
    fireEvent.change(textarea, { target: { value: "a".repeat(30) } });
    expect(screen.getByRole("button", { name: /extract brief/i })).toBeDisabled();

    fireEvent.click(screen.getByRole("tab", { name: "Upload requirements" }));
    expect(screen.getByRole("button", { name: /choose a document/i })).not.toBeDisabled();
  });

  it("disables Describe when Ollama itself is unreachable", async () => {
    setLocalModelStatusOverride({ ollama_reachable: false, model_present: false });
    renderHome();

    expect(await screen.findByText("Local model unavailable")).toBeInTheDocument();
    expect(screen.getByText(/isn't reachable/i)).toBeInTheDocument();

    const textarea = await screen.findByLabelText("Project description");
    fireEvent.change(textarea, { target: { value: "a".repeat(30) } });
    expect(screen.getByRole("button", { name: /extract brief/i })).toBeDisabled();
  });

  it("leaves Describe enabled at 20+ characters when the local model is reachable and present", async () => {
    renderHome();
    const textarea = await screen.findByLabelText("Project description");
    expect(screen.queryByText("Local model unavailable")).not.toBeInTheDocument();

    fireEvent.change(textarea, { target: { value: "a".repeat(20) } });
    expect(await screen.findByRole("button", { name: /extract brief/i })).not.toBeDisabled();
  });
});
