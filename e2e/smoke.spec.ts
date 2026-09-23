import { expect, test, type Page } from "@playwright/test";

/**
 * AC-18 smoke test, run against the built web bundle (`vite preview`). There is
 * no Tauri shim: `src/lib/ipc.ts` picks `mockApi` from `src/lib/ipc-mock.ts`
 * whenever `window.__TAURI_INTERNALS__` is absent, which is always the case in
 * a plain browser — so the whole app runs end to end against realistic,
 * seeded in-memory data.
 *
 * The mock keeps its state in memory for the lifetime of the page, so once
 * configured it stays configured across in-app (client-side) navigation — but
 * a full browser navigation (`page.goto`) reloads the bundle and resets it.
 * Only the very first navigation in each test uses `page.goto`; every screen
 * change after that goes through the app's own nav links or buttons.
 *
 * The mock requires an OpenRouter API key (for `run_decisions`) and a reviewer
 * name (for `review`) before those steps succeed, so both are set through the
 * Settings screen up front.
 */
async function configureSettings(page: Page) {
  await page.getByRole("link", { name: "Settings" }).click();
  await expect(page).toHaveURL(/\/settings$/);

  await page.getByLabel("API key").fill("sk-or-test-key");
  await page.getByRole("button", { name: "Save key" }).click();
  await expect(page.getByText("Key is set ✓")).toBeVisible();

  await page.getByLabel("Your name").fill("A. Fernando");
  await page.getByRole("button", { name: "Save settings" }).click();
  await expect(page.getByText("Settings saved.")).toBeVisible();

  await page.getByRole("link", { name: "Home" }).click();
  await expect(page).toHaveURL(/\/$/);
}

test.describe("BISTEC Architect smoke test (AC-18)", () => {
  test("Describe → Brief → Run → Report → Accept → export enabled", async ({ page }) => {
    await page.goto("/");
    await configureSettings(page);

    // ---- Home: Describe tab, type a project description, start ----
    await page.getByRole("tab", { name: "Describe" }).click();

    const description =
      "Build a customer self-service portal for our insurance clients, roughly 20,000 " +
      "policyholders, signing in via our Microsoft 365 tenant. Budget is tight and our " +
      "team is all .NET. Needs to handle claims, policy documents, billing, and GDPR data.";
    await page.getByLabel("Project description").fill(description);
    await page.getByRole("button", { name: "Extract brief" }).click();

    // ---- Data notice: acknowledge on first use ----
    const notice = page.getByRole("dialog", { name: "Data notice" });
    await expect(notice).toBeVisible();
    await notice.getByRole("button", { name: "Acknowledge & continue" }).click();
    await expect(notice).not.toBeVisible();

    // ---- Brief screen: the extracted brief is shown ----
    await expect(page).toHaveURL(/\/session\/[^/]+\/brief$/);
    await expect(page.getByLabel("Summary")).not.toBeEmpty();
    const confirmButton = page.getByRole("button", { name: "Confirm brief & run decisions" });
    await expect(confirmButton).toBeEnabled();
    await confirmButton.click();

    // ---- Run screen: stage progress ----
    await expect(page).toHaveURL(/\/session\/[^/]+\/run$/);
    await expect(page.getByRole("heading", { name: "Running decisions" })).toBeVisible();
    await expect(page.getByText("Gates", { exact: true })).toBeVisible();
    await expect(page.getByText("Platforms", { exact: true })).toBeVisible();
    await expect(page.getByText("Applicability", { exact: true })).toBeVisible();
    await expect(page.getByText("Decisions", { exact: true })).toBeVisible();

    // ---- Report screen: at least one decision card ----
    await expect(page).toHaveURL(/\/session\/[^/]+\/report$/, { timeout: 15_000 });
    await expect(page.getByRole("heading", { name: "Report summary" })).toBeVisible();
    const decisionHeadings = page.getByRole("heading", { level: 3 });
    await expect(decisionHeadings).not.toHaveCount(0);

    // ---- Open review on an unreviewed card and accept it ----
    const card = page
      .locator('[data-slot="card"]')
      .filter({ has: page.getByRole("heading", { level: 3, name: "Backend platform" }) });
    await expect(card.getByText("Proposed (AI) — pending approval")).toBeVisible();
    await card.getByRole("button", { name: "Review" }).click();

    const reviewDialog = page.getByRole("dialog", { name: "Review — Backend platform" });
    await expect(reviewDialog).toBeVisible();
    await expect(reviewDialog.getByText("Reviewing as A. Fernando")).toBeVisible();
    await reviewDialog.getByRole("button", { name: "Accept" }).click();
    await expect(reviewDialog).not.toBeVisible();

    await expect(card.getByText("Accepted", { exact: true })).toBeVisible();

    // ---- Export ADRs is enabled ----
    await expect(page.getByRole("button", { name: "Export ADRs" })).toBeEnabled();
  });
});

test.describe("Settings health panel", () => {
  test("shows connection status from the mock's default health check", async ({ page }) => {
    await page.goto("/settings");

    await expect(page.getByText("Connections", { exact: true })).toBeVisible();
    await page.getByRole("button", { name: "Check connections" }).click();

    // Mock default health: Ollama reachable and the model present (both "OK"),
    // no API key set yet ("Fail", with guidance to set one).
    await expect(page.getByText("Ollama reachable")).toBeVisible();
    await expect(page.getByText(/Model present \(/)).toBeVisible();
    await expect(page.getByText("OK", { exact: true })).toHaveCount(2);
    await expect(page.getByText("Fail", { exact: true })).toHaveCount(1);
    await expect(page.getByText("No OpenRouter API key is set yet.")).toBeVisible();
  });
});
