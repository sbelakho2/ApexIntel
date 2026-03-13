const fs = require('node:fs');
const path = require('node:path');
const { test, expect } = require('@playwright/test');

const currentRerunDir = path.join(__dirname, 'current-rerun', 'mobile');

const credentials = {
  username: process.env.PLAYWRIGHT_USER || 'playwright',
  password: process.env.PLAYWRIGHT_PASSWORD || 'playwright',
};

const routes = [
  { name: 'mobile-dashboard-chart-page.png', path: '/', heading: 'Overview' },
  { name: 'mobile-warnings-chart-page.png', path: '/warnings', heading: 'Warning Center' },
  { name: 'mobile-insights-chart-page.png', path: '/insights', heading: 'Insight Feed' },
  { name: 'mobile-recipes-chart-page.png', path: '/recipes', heading: 'Recipe Manager' },
  { name: 'mobile-companies-chart-page.png', path: '/companies', heading: 'Company Browser' },
  { name: 'mobile-competitors-chart-page.png', path: '/competitors', heading: 'Competitor Dashboard' },
  { name: 'mobile-graph-chart-page.png', path: '/graph', heading: 'Graph Explorer' },
];

function ensureArtifactDir() {
  fs.mkdirSync(currentRerunDir, { recursive: true });
}

async function login(page) {
  await page.goto('/login', { waitUntil: 'networkidle' });
  await page.getByLabel(/operator id/i).fill(credentials.username);
  await page.getByLabel(/access key/i).fill(credentials.password);
  await page.locator('#login-submit, button[type="submit"]').first().click();
  await expect(page).not.toHaveURL(/\/login$/);
}

test.use({
  viewport: {
    width: 390,
    height: 844,
  },
});

test.beforeAll(() => {
  ensureArtifactDir();
});

test.beforeEach(async ({ page }) => {
  await login(page);
});

for (const route of routes) {
  test(`renders ${route.path} with mobile chart parity`, async ({ page }) => {
    await page.goto(route.path, { waitUntil: 'networkidle' });
    await expect(page.getByRole('heading', { level: 1, name: route.heading })).toBeVisible();
    await expect(page.locator('canvas')).toHaveCount(0);

    const chartCount = await page
      .locator('.apex-viz-frame, .apex-line-chart, .apex-compare-chart, .community-graph, svg')
      .count();
    expect(chartCount).toBeGreaterThan(0);

    const metrics = await page.evaluate(() => ({
      width: window.innerWidth,
      body: document.body.scrollWidth,
      main: document.querySelector('main')?.scrollWidth ?? 0,
    }));
    expect(metrics.body).toBe(metrics.width);
    expect(metrics.main).toBe(metrics.width);

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