const { defineConfig } = require('@playwright/test');

module.exports = defineConfig({
  testDir: './e2e',
  timeout: 60_000,
  retries: process.env.CI ? 2 : 0,
  fullyParallel: false,
  reporter: 'list',
  use: {
    baseURL: process.env.PLAYWRIGHT_BASE_URL || 'http://127.0.0.1:8080',
    headless: true,
    viewport: {
      width: 1440,
      height: 1800,
    },
  },
  webServer: {
    command: 'cd crates/frontend && trunk serve --port 8080',
    port: 8080,
    timeout: 120_000,
    reuseExistingServer: !process.env.CI,
  },
});
