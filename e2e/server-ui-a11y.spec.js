// @ts-check
// Accessibility gate for the primary server-UI surfaces (P0).
//
// Runs axe-core against /, /warnings, and /companies and fails on serious or
// critical violations. Moderate/minor findings are reported but do not fail the
// build (they are tracked separately).
const { test, expect } = require('@playwright/test');
const AxeBuilder = require('@axe-core/playwright').default;
const { seedDatabase, login } = require('./helpers/server-ui-fixtures.cjs');

const ROUTES = ['/', '/warnings', '/companies'];

test.beforeAll(async () => {
  await seedDatabase();
});

test.describe('server UI — accessibility (axe-core)', () => {
  for (const route of ROUTES) {
    test(`axe: ${route} has no serious or critical violations`, async ({ page }) => {
      await login(page);
      await page.goto(route, { waitUntil: 'domcontentloaded' });
      // The SSE stream keeps the network busy, so `networkidle` never settles;
      // allow hydration/htmx to finish instead.
      await page.waitForTimeout(500);

      const results = await new AxeBuilder({ page })
        .withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa'])
        .analyze();

      const blocking = results.violations
        .filter((violation) => violation.impact === 'serious' || violation.impact === 'critical')
        .map((violation) => ({
          id: violation.id,
          impact: violation.impact,
          help: violation.help,
          nodes: violation.nodes.map((node) => node.target.join(' ')).slice(0, 5),
        }));

      expect(blocking, `axe violations on ${route}: ${JSON.stringify(blocking, null, 2)}`).toEqual(
        []
      );
    });
  }
});
