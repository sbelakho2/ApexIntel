const { defineConfig } = require('@playwright/test');

module.exports = defineConfig({
  testDir: './e2e/parity',
  timeout: 60_000,
  fullyParallel: false,
  reporter: 'list',
  snapshotPathTemplate: '{testDir}/baseline/{testFilePath}/{arg}{ext}',
  use: {
    baseURL: process.env.PLAYWRIGHT_BASE_URL || 'http://127.0.0.1:8080',
    headless: true,
    viewport: {
      width: 1440,
      height: 1800,
    },
  },
});