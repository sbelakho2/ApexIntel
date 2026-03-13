const { test, expect } = require('@playwright/test');

const routes = [
  ['dashboard', '/'],
  ['warnings', '/warnings'],
  ['insights', '/insights'],
  ['recipes', '/recipes'],
  ['companies', '/companies'],
  ['competitors', '/competitors'],
  ['graph', '/graph'],
];

async function login(page) {
  await page.goto('/login', { waitUntil: 'networkidle' });
  await page.fill('input[name="username"]', process.env.PLAYWRIGHT_USER || 'playwright');
  await page.fill('input[name="password"]', process.env.PLAYWRIGHT_PASSWORD || 'playwright');
  await Promise.all([
    page.waitForURL('**/', { timeout: 15000 }),
    page.click('button[type="submit"]'),
  ]);
}

test.describe('chart pages', () => {
  test.beforeEach(async ({ page }) => {
    await login(page);
  });

  for (const [name, path] of routes) {
    test(`${name} renders charts without canvas placeholders`, async ({ page }) => {
      await page.goto(path, { waitUntil: 'networkidle' });

      await expect(page.locator('h1').first()).toBeVisible();
      await expect(page.locator('canvas')).toHaveCount(0);

      const chartCount = await page
        .locator('.apex-viz-frame, .apex-line-chart, .apex-compare-chart, .community-graph, svg')
        .count();

      expect(chartCount).toBeGreaterThan(0);
    });
  }
});