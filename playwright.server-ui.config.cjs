// Playwright configuration for the SERVER-rendered UI contract specs.
//
// Unlike playwright.config.cjs (the wasm/trunk experiment at :8080), these
// specs run against a live `apex-api` instance serving the Askama UI — the same
// server the CI `e2e-server-ui` step starts at :9095. The server is started by
// the caller (CI step or `npm run test:server-ui:specs` with a local API), so
// there is no webServer here.
//
// Visual baselines are generated on Linux (CI) and carry Playwright's platform
// suffix; regenerate with `npm run test:server-ui:specs:update` inside the CI
// container environment.
const { defineConfig } = require('@playwright/test');

module.exports = defineConfig({
  testDir: './e2e',
  testMatch: /server-ui-.*\.spec\.js$/,
  timeout: 90_000,
  retries: process.env.CI ? 1 : 0,
  fullyParallel: false,
  // Deterministic ordering: the acknowledge task flow changes warning state,
  // and the visual specs re-seed, so files must not interleave.
  workers: 1,
  reporter: 'list',
  expect: {
    toHaveScreenshot: {
      maxDiffPixelRatio: 0.02,
      animations: 'disabled',
    },
  },
  use: {
    baseURL: process.env.PLAYWRIGHT_BASE_URL || process.env.BASE_URL || 'http://127.0.0.1:9095',
    headless: true,
    viewport: { width: 1440, height: 900 },
    actionTimeout: 15_000,
    navigationTimeout: 30_000,
  },
});
