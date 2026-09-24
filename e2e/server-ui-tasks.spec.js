// @ts-check
// Task-based server-UI contract flows (P0 gate).
//
// Each test is written the way an analyst uses the product, with an explicit
// interaction budget, against the seeded fixture corpus in
// e2e/helpers/server-ui-fixtures.cjs.
const { test, expect } = require('@playwright/test');
const { SEED, seedDatabase, login } = require('./helpers/server-ui-fixtures.cjs');

test.beforeAll(async () => {
  await seedDatabase();
});

test.describe('server UI — analyst task flows', () => {
  test('finds a company in at most two interactions', async ({ page }) => {
    await login(page);
    await page.goto('/companies', { waitUntil: 'domcontentloaded' });

    // Interaction 1: type the company name. Interaction 2: submit (Enter).
    await page.locator('#companies-search').fill(SEED.companies[0].name);
    await page.locator('#companies-search').press('Enter');

    const results = page.locator('#main-results');
    await expect(results).toContainText(SEED.companies[0].name);
    await expect(results).not.toContainText(SEED.companies[1].name);
  });

  test('opens a warning and sees its evidence', async ({ page }) => {
    await login(page);
    await page.goto('/warnings', { waitUntil: 'domcontentloaded' });

    const warning = SEED.warnings[0];
    await page.locator('tr[data-row-link]', { hasText: warning.title }).click();
    await page.waitForURL(`**/warnings/${warning.id}`);

    await expect(page.getByRole('heading', { name: 'Evidence', exact: true })).toBeVisible();
    await expect(page.getByText('example.test').first()).toBeVisible();
  });

  test('acknowledges a warning', async ({ page }) => {
    await login(page);
    await page.goto('/warnings', { waitUntil: 'domcontentloaded' });

    const warning = SEED.warnings[1];
    await page.locator('tr[data-row-link]', { hasText: warning.title }).click();
    await page.waitForURL(`**/warnings/${warning.id}`);

    await page.getByRole('button', { name: 'Acknowledge', exact: true }).click();
    await expect(page.locator('#ack-status')).toContainText(/acknowledged/i, {
      timeout: 15_000,
    });
  });

  test('opens an investigation workspace', async ({ page }) => {
    await login(page);
    await page.goto('/workspaces', { waitUntil: 'domcontentloaded' });

    await page.locator(`a[href="/workspaces/${SEED.workspace.id}"]`).first().click();
    await page.waitForURL(`**/workspaces/${SEED.workspace.id}`);

    await expect(
      page.getByRole('heading', { name: SEED.workspace.name, exact: true })
    ).toBeVisible();
  });
});
