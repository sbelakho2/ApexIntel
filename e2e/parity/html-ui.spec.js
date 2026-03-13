const fs = require('node:fs');
const path = require('node:path');
const { test, expect } = require('@playwright/test');

const currentRerunDir = path.join(__dirname, 'current-rerun');

const credentials = {
  username: process.env.PLAYWRIGHT_USER || 'playwright',
  password: process.env.PLAYWRIGHT_PASSWORD || 'playwright',
};

const replacementRoutes = [
  {
    name: 'html-dashboard-page.png',
    path: '/',
    heading: 'Overview',
    minCards: 6,
  },
  {
    name: 'html-warnings-list.png',
    path: '/warnings',
    heading: 'Warning Center',
    minCards: 4,
  },
  {
    name: 'html-graph-page.png',
    path: '/graph',
    heading: 'Graph Explorer',
    minCards: 9,
  },
  {
    name: 'html-admin-page.png',
    path: '/admin',
    heading: 'Admin Dashboard',
    minCards: 7,
  },
];

const legacyRedirects = [
  {
    from: '/wasm',
    to: '/',
    heading: 'Overview',
  },
  {
    from: '/wasm/warnings',
    to: '/warnings',
    heading: 'Warning Center',
  },
  {
    from: '/wasm/graph',
    to: '/graph',
    heading: 'Graph Explorer',
  },
  {
    from: '/wasm/calibration',
    to: '/admin',
    heading: 'Admin Dashboard',
  },
];

function ensureArtifactDir() {
  fs.mkdirSync(currentRerunDir, { recursive: true });
}

async function login(page) {
  await page.goto('/login', { waitUntil: 'networkidle' });
  await page.getByLabel(/operator id/i).fill(credentials.username);
  await page.getByLabel(/access key/i).fill(credentials.password);
  await page.locator('#login-submit').click();
  await expect(page).not.toHaveURL(/\/login$/);
}

test.beforeAll(() => {
  ensureArtifactDir();
});

test.beforeEach(async ({ page }) => {
  await login(page);
});

for (const route of replacementRoutes) {
  test(`renders ${route.path} with Rams parity`, async ({ page }) => {
    await page.goto(route.path, { waitUntil: 'networkidle' });
    await expect(page.getByRole('heading', { level: 1, name: route.heading })).toBeVisible();
    await expect(page.locator('.apex-card').first()).toBeVisible();

    const cardCount = await page.locator('.apex-card').count();
    expect(cardCount).toBeGreaterThanOrEqual(route.minCards);

    const rerunPath = path.join(currentRerunDir, route.name);
    await page.screenshot({
      path: rerunPath,
      fullPage: true,
      animations: 'disabled',
    });

    await expect(page).toHaveScreenshot(route.name, {
      fullPage: true,
      animations: 'disabled',
    });
  });
}

test('legacy wasm routes redirect to the HTML replacements', async ({ page, baseURL }) => {
  for (const route of legacyRedirects) {
    await page.goto(route.from, { waitUntil: 'networkidle' });
    await expect(page).toHaveURL(new URL(route.to, baseURL).toString());
    await expect(page.getByRole('heading', { level: 1, name: route.heading })).toBeVisible();
  }
});