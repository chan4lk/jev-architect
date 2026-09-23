import { beforeEach, describe, expect, it } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import { Settings } from "./Settings";
import { resetMock } from "@/lib/ipc-mock";

function renderSettings() {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={queryClient}>
      <Settings />
    </QueryClientProvider>,
  );
}

describe("Settings — API key", () => {
  beforeEach(() => resetMock());

  it("never renders the key value after it is saved", async () => {
    renderSettings();
    const input = (await screen.findByLabelText("API key")) as HTMLInputElement;
    const secretKey = "sk-or-super-secret-value-12345";

    fireEvent.change(input, { target: { value: secretKey } });
    fireEvent.click(screen.getByRole("button", { name: /save key/i }));

    await waitFor(() => expect(input.value).toBe(""));
    expect(await screen.findByText("Key is set ✓")).toBeInTheDocument();
    expect(document.body.textContent).not.toContain(secretKey);
  });
});
