import { defineConfig, devices } from "@playwright/test";

/**
 * Playwright config for the smoke test (AC-18). Runs against the built
 * (`vite preview`) web bundle — no Tauri shim is needed, because `src/lib/ipc.ts`
 * falls back to the in-memory mock IPC (`src/lib/ipc-mock.ts`) whenever
 * `window.__TAURI_INTERNALS__` is absent, which is always true in a plain
 * browser.
 */
export default defineConfig({
  testDir: "e2e",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  reporter: "list",

  use: {
    baseURL: "http://localhost:4173",
    trace: "on-first-retry",
  },

  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
  ],

  webServer: {
    command: "pnpm build && pnpm preview --port 4173 --strictPort",
    url: "http://localhost:4173",
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
  },
});
