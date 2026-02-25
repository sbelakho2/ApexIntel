import { defineConfig, devices } from '@playwright/test';

export default defineConfig({
  testDir: './e2e',
  fullyParallel: true,
  retries: process.env.CI ? 2 : 0,
  workers: process.env.CI ? 1 : undefined,
  reporter: [['list'], ['html', { outputFolder: 'playwright-report' }]],
  use: {
    baseURL: 'http://127.0.0.1:3006',
    trace: 'on-first-retry',
    screenshot: 'only-on-failure',
  },
  projects: [
    {
      name: 'chromium-desktop',
      use: {
        ...devices['Desktop Chrome'],
        viewport: { width: 1536, height: 960 },
      },
    },
  ],
  webServer: {
    command: 'npm run dev -- -p 3006',
    cwd: __dirname,
    url: 'http://127.0.0.1:3006',
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
  },
});
