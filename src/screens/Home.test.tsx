import { beforeEach, describe, expect, it } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import { Home } from "./Home";
import { resetMock } from "@/lib/ipc-mock";

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
