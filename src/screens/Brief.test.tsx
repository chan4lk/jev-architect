import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import { Brief } from "./Brief";
import { mockApi, resetMock } from "@/lib/ipc-mock";

function renderBrief(sessionId: string) {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={queryClient}>
      <MemoryRouter initialEntries={[`/session/${sessionId}/brief`]}>
        <Routes>
          <Route path="/session/:id/brief" element={<Brief />} />
          <Route path="/session/:id/run" element={<div>Run screen</div>} />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

describe("Brief screen", () => {
  beforeEach(() => resetMock());

  it("editing a context select and saving calls updateBrief with the edited value", async () => {
    const spy = vi.spyOn(mockApi, "updateBrief");
    renderBrief("session-seed-brief");

    const scaleTrigger = await screen.findByLabelText("Scale");
    fireEvent.click(scaleTrigger);
    const largeOption = await screen.findByRole("option", { name: /large — 100k\+/i });
    // base-ui's Select only commits a click-driven selection when it was preceded by a
    // real pointerdown (to distinguish it from a stray synthetic click); mirror that here.
    fireEvent.pointerDown(largeOption);
    fireEvent.click(largeOption);

    fireEvent.click(screen.getByRole("button", { name: /save changes/i }));

    await waitFor(() => expect(spy).toHaveBeenCalledTimes(1));
    const [, brief] = spy.mock.calls[0];
    expect(brief.context.scale).toBe("large");
  });

  it("disables confirm while the summary is empty", async () => {
    renderBrief("session-seed-brief");
    const summary = await screen.findByLabelText("Summary");
    expect(screen.getByRole("button", { name: /confirm brief/i })).not.toBeDisabled();

    fireEvent.change(summary, { target: { value: "" } });
    expect(screen.getByRole("button", { name: /confirm brief/i })).toBeDisabled();
  });

  it("confirming a dirty brief saves it first, then confirms, then navigates to run", async () => {
    const updateSpy = vi.spyOn(mockApi, "updateBrief");
    const confirmSpy = vi.spyOn(mockApi, "confirmBrief");
    renderBrief("session-seed-brief");

    const summary = await screen.findByLabelText("Summary");
    fireEvent.change(summary, { target: { value: "An updated summary for this session." } });
    expect(await screen.findByText("Unsaved changes")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /confirm brief/i }));

    expect(await screen.findByText("Run screen")).toBeInTheDocument();
    expect(updateSpy).toHaveBeenCalledTimes(1);
    expect(updateSpy.mock.calls[0][1].summary).toBe("An updated summary for this session.");
    expect(confirmSpy).toHaveBeenCalledTimes(1);
  });

  it("shows document sections and citation chips for an upload session", async () => {
    renderBrief("session-seed-brief");

    expect(await screen.findByText("Document sections")).toBeInTheDocument();
    expect(screen.getAllByText("S1").length).toBeGreaterThan(0);

    const citationChip = screen.getByRole("button", { name: "Jump to section S1" });
    fireEvent.click(citationChip);

    expect(
      await screen.findByText(/Policyholders must sign in using single sign-on/i),
    ).toBeInTheDocument();
  });
});
