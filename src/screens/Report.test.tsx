import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import { Report } from "./Report";
import { resetMock } from "@/lib/ipc-mock";
import { api } from "@/lib/ipc";

const SEED_SESSION_ID = "session-seed-report";

function renderReport(sessionId = SEED_SESSION_ID) {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={queryClient}>
      <MemoryRouter initialEntries={[`/session/${sessionId}/report`]}>
        <Routes>
          <Route path="/session/:id/report" element={<Report />} />
          <Route path="/settings" element={<div>Settings screen</div>} />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

async function setReviewerName(name: string) {
  const current = await api.getSettings();
  await api.saveSettings({ ...current, reviewer_name: name });
}

describe("Report screen", () => {
  beforeEach(() => resetMock());

  it("renders all decision cards from the seeded report with ring badges and reason text", async () => {
    renderReport();

    const headings = await screen.findAllByRole("heading", { level: 3 });
    expect(headings).toHaveLength(9);

    expect(screen.getAllByText("Adopt").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Trial").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Hold").length).toBeGreaterThan(0);

    expect(screen.getAllByText("Low confidence").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Scores disagree with Jev's choice").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Close call").length).toBeGreaterThan(0);
    expect(screen.getAllByText("BISTEC says avoid").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Possible prompt injection").length).toBeGreaterThan(0);
  });

  it('"Needs architect" filter shows only those decisions', async () => {
    renderReport();
    await screen.findAllByRole("heading", { level: 3 });

    fireEvent.click(screen.getByRole("tab", { name: /needs architect/i }));

    await waitFor(() => {
      const headings = screen.getAllByRole("heading", { level: 3 });
      expect(headings).toHaveLength(4);
    });
  });

  it('Accept records a review and the status becomes "Accepted" (AC-13 UI)', async () => {
    await setReviewerName("Ada Reviewer");
    renderReport();

    const headings = await screen.findAllByRole("heading", { level: 3 });
    // index 1 = "Backend platform", unreviewed in the seed.
    const card = headings[1].closest('[data-slot="card"]') as HTMLElement;
    fireEvent.click(within(card).getByRole("button", { name: "Review" }));

    const dialog = await screen.findByRole("dialog");
    fireEvent.click(within(dialog).getByRole("button", { name: "Accept" }));

    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(within(card).getByText("Accepted")).toBeInTheDocument();
  });

  it("Override without a reason shows the validation message and does not call review", async () => {
    await setReviewerName("Ada Reviewer");
    const reviewSpy = vi.spyOn(api, "review");
    renderReport();

    const headings = await screen.findAllByRole("heading", { level: 3 });
    const card = headings[1].closest('[data-slot="card"]') as HTMLElement;
    fireEvent.click(within(card).getByRole("button", { name: "Review" }));

    const dialog = await screen.findByRole("dialog");
    fireEvent.click(within(dialog).getByRole("button", { name: "Override" }));
    fireEvent.click(within(dialog).getByLabelText(/node\.js/i));
    fireEvent.click(within(dialog).getByRole("button", { name: "Confirm override" }));

    expect(await within(dialog).findByText(/a reason is required/i)).toBeInTheDocument();
    expect(reviewSpy).not.toHaveBeenCalled();
  });

  it('Override with option and reason updates status to "Accepted (override)"', async () => {
    await setReviewerName("Ada Reviewer");
    renderReport();

    const headings = await screen.findAllByRole("heading", { level: 3 });
    const card = headings[1].closest('[data-slot="card"]') as HTMLElement;
    fireEvent.click(within(card).getByRole("button", { name: "Review" }));

    const dialog = await screen.findByRole("dialog");
    fireEvent.click(within(dialog).getByRole("button", { name: "Override" }));
    fireEvent.click(within(dialog).getByLabelText(/node\.js/i));
    fireEvent.change(within(dialog).getByLabelText("Reason"), {
      target: { value: "Team already knows Node for this service." },
    });
    fireEvent.click(within(dialog).getByRole("button", { name: "Confirm override" }));

    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(within(card).getByText("Accepted (override)")).toBeInTheDocument();
  });

  it("disables review actions and links to Settings when the reviewer name is blank", async () => {
    renderReport();

    const reviewButtons = await screen.findAllByRole("button", { name: "Review" });
    fireEvent.click(reviewButtons[0]);

    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByRole("button", { name: "Accept" })).toBeDisabled();
    expect(within(dialog).getByRole("button", { name: "Override" })).toBeDisabled();
    expect(within(dialog).getByRole("button", { name: "Reject" })).toBeDisabled();
    expect(within(dialog).getByRole("link", { name: /go to settings/i })).toBeInTheDocument();
  });

  it("export ADRs calls exportAdrs with the picked folder and warns about unreviewed decisions", async () => {
    const pickFolderSpy = vi.spyOn(api, "pickFolder");
    const exportAdrsSpy = vi.spyOn(api, "exportAdrs");
    renderReport();

    await screen.findAllByRole("heading", { level: 3 });
    fireEvent.click(screen.getByRole("button", { name: "Export ADRs" }));

    const dialog = await screen.findByRole("dialog");
    // backend-platform, auth-b2c, document-db, css, full-text-search have no review in the seed.
    expect(
      within(dialog).getByText(/5 decisions will export as Proposed \(AI\) — pending approval/i),
    ).toBeInTheDocument();

    fireEvent.click(within(dialog).getByRole("button", { name: /choose folder & export/i }));

    await waitFor(() =>
      expect(exportAdrsSpy).toHaveBeenCalledWith(SEED_SESSION_ID, "/mock/exports"),
    );
    expect(pickFolderSpy).toHaveBeenCalled();
  });
});
