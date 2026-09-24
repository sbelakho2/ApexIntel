// @ts-check
// Visual regression gate for the primary server-UI surfaces (P0).
//
// Screenshots are taken at desktop / tablet / mobile for /, /warnings, and
// /companies against the deterministic seed corpus. Baseline images are
// Linux-rendered (CI container); regenerate with
// `npm run test:server-ui:specs:update` in the same environment.
const { test, expect } = require('@playwright/test');
const { seedDatabase, login } = require('./helpers/server-ui-fixtures.cjs');

const VIEWPORTS = [
  { name: 'desktop', width: 1440, height: 900 },
  { name: 'tablet', width: 834, height: 1112 },
  { name: 'mobile', width: 390, height: 844 },
];

const ROUTES = [
  { name: 'dashboard', path: '/' },
  { name: 'warnings', path: '/warnings' },
  { name: 'companies', path: '/companies' },
];

test.beforeAll(async () => {
  // Re-seed so the acknowledged-warning task flow cannot change these pixels.
  await seedDatabase();
});

test.describe('server UI — visual snapshots', () => {
  for (const viewport of VIEWPORTS) {
    for (const route of ROUTES) {
      test(`${route.name} @ ${viewport.name}`, async ({ page }) => {
        await page.setViewportSize({ width: viewport.width, height: viewport.height });
        await login(page);
        await page.goto(route.path, { waitUntil: 'domcontentloaded' });
        await page.waitForTimeout(500);

        // Time-windowed trend/hourly charts label their windows relative to
        // "now", so mask only those volatile labels. Chart geometry — bars,
        // values, segments, frames — stays in the comparison.
        const volatileLabels = page.locator('.apex-viz-label');
        const mask = (await volatileLabels.count()) > 0 ? [volatileLabels] : [];

        await expect(page).toHaveScreenshot(`${route.name}-${viewport.name}.png`, {
          fullPage: true,
          mask,
        });
      });
    }
  }
});
