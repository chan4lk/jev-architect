import { beforeEach, describe, expect, it } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import { Run } from "./Run";
import { mockApi, resetMock, failNextRunDecisions } from "@/lib/ipc-mock";

function renderRun(sessionId: string) {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={queryClient}>
      <MemoryRouter initialEntries={[`/session/${sessionId}/run`]}>
        <Routes>
          <Route path="/session/:id/run" element={<Run />} />
          <Route path="/session/:id/report" element={<div>Report screen</div>} />
          <Route path="/session/:id/brief" element={<div>Brief screen</div>} />
          <Route path="/settings" element={<div>Settings screen</div>} />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

async function setUpConfirmedSession(): Promise<string> {
  const view = await mockApi.startDescribe(
    "Build a small internal tool for tracking team on-call rotations and escalation.",
  );
  await mockApi.confirmBrief(view.session.id);
  await mockApi.setApiKey("sk-or-test-key");
  await mockApi.ackDataNotice();
  return view.session.id;
}

describe("Run screen", () => {
  beforeEach(() => resetMock());

  it("shows progress stages from mock events and navigates to the report on completion", async () => {
    const id = await setUpConfirmedSession();
    renderRun(id);

    expect(await screen.findByText("Gates")).toBeInTheDocument();

    // The mock emits a done/total progress bar during the "decisions" stage.
    await waitFor(() => expect(screen.getByText(/\d+ \/ \d+ decisions/)).toBeInTheDocument(), {
      timeout: 3000,
    });

    expect(await screen.findByText("Report screen", {}, { timeout: 3000 })).toBeInTheDocument();
  });

  it("shows an error with Retry on failure, and retry resumes to completion", async () => {
    const id = await setUpConfirmedSession();
    failNextRunDecisions("platforms", { code: "jev", message: "Simulated Jev failure." });
    renderRun(id);

    expect(
      await screen.findByText(/failed at stage "platforms"/i, {}, { timeout: 3000 }),
    ).toBeInTheDocument();
    expect(screen.getByText("Simulated Jev failure.")).toBeInTheDocument();

    const retryButton = screen.getByRole("button", { name: /retry/i });
    fireEvent.click(retryButton);

    expect(await screen.findByText("Report screen", {}, { timeout: 3000 })).toBeInTheDocument();
  });
});
