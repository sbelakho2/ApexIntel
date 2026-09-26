// @ts-check
// Task-based server-UI contract flows (P0 gate).
//
// Each test is written the way an analyst uses the product, with an explicit
// interaction budget, against the seeded fixture corpus in
// e2e/helpers/server-ui-fixtures.cjs. Budgets count user actions only
// (clicks, fills, key presses); `goto`/`login` are setup.
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

  test('opens evidence from a warning in at most two actions', async ({ page }) => {
    await login(page);
    await page.goto('/warnings', { waitUntil: 'domcontentloaded' });

    const warning = SEED.warnings[0];
    const budget = [];

    budget.push('open warning from signal list');
    await page.locator('tr[data-row-link]', { hasText: warning.title }).click();
    await page.waitForURL(`**/warnings/${warning.id}`);

    const sourceLink = page.locator('[data-claim-source]').first();
    await expect(sourceLink).toBeVisible();
    const evidenceHref = await sourceLink.getAttribute('href');
    budget.push('open claim source');
    const [popup] = await Promise.all([page.waitForEvent('popup'), sourceLink.click()]);

    // The evidence source is external (example.test); assert the link target
    // and that following it opened the source in a new page.
    expect(evidenceHref).toContain('example.test');
    expect(popup).toBeTruthy();
    expect(budget.length).toBeLessThanOrEqual(2);
    await popup.close();
  });

  test('subscribes to entity alerts in at most two actions', async ({ page }) => {
    await login(page);
    await page.goto('/companies', { waitUntil: 'domcontentloaded' });

    const company = SEED.companies[0];
    const budget = [];

    budget.push('open company dossier');
    await page.locator('tr[data-row-link]', { hasText: company.name }).click();
    await page.waitForURL(`**/companies/${company.id}`);

    budget.push('watch alerts');
    await page.locator('[data-alert-subscription-save]').click();
    await expect(page.locator('[data-alert-subscription-state]')).toHaveText(/Watching/, {
      timeout: 15_000,
    });
    expect(budget.length).toBeLessThanOrEqual(2);
  });

  test('starts an investigation from a signal in at most two actions', async ({ page }) => {
    await login(page);
    await page.goto('/warnings', { waitUntil: 'domcontentloaded' });

    const warning = SEED.warnings[0];
    const budget = [];

    budget.push('open warning from signal list');
    await page.locator('tr[data-row-link]', { hasText: warning.title }).click();
    await page.waitForURL(`**/warnings/${warning.id}`);

    budget.push('start investigation');
    await page.locator('[data-action="start-investigation"]').click();
    await page.waitForURL(/\/workspaces\/[0-9a-f-]{36}$/);
    await expect(page.locator('h1')).toContainText(/Investigate:/);

    expect(budget.length).toBeLessThanOrEqual(2);
  });

  test('finds a person from a company dossier in at most two actions', async ({ page }) => {
    await login(page);
    await page.goto('/companies', { waitUntil: 'domcontentloaded' });

    const company = SEED.companies[0];
    const person = SEED.persons[0];
    const budget = [];

    budget.push('open company dossier');
    await page.locator('tr[data-row-link]', { hasText: company.name }).click();
    await page.waitForURL(`**/companies/${company.id}`);

    budget.push('open person from dossier');
    await page.locator('[data-person-link]', { hasText: person.name }).first().click();
    await page.waitForURL(`**/persons/${person.id}`);
    await expect(page.locator('h1', { hasText: person.name })).toBeVisible();

    expect(budget.length).toBeLessThanOrEqual(2);
  });

  test('bookmarks an insight', async ({ page }) => {
    await login(page);
    await page.goto('/insights', { waitUntil: 'domcontentloaded' });

    const insight = SEED.insight;
    await page.locator(`a[href="/insights/${insight.id}"]`).first().click();
    await page.waitForURL(`**/insights/${insight.id}`);

    await page.getByRole('button', { name: 'Bookmark', exact: true }).click();
    await expect(
      page.locator('#bookmark-status button[title="Remove bookmark"]')
    ).toBeVisible({ timeout: 15_000 });
  });

  test('traces a claim to its source', async ({ page }) => {
    await login(page);
    const insight = SEED.insight;
    await page.goto(`/insights/${insight.id}`, { waitUntil: 'domcontentloaded' });

    const citation = page.locator('a[title="Evidence source 1"]').first();
    await expect(citation).toBeVisible();
    await citation.click();

    expect(page.url()).toContain('#source-1');
    const source = page.locator('#source-1 a[href]').first();
    await expect(source).toHaveAttribute('href', insight.evidenceUrl);
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
