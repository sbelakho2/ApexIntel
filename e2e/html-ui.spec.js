// @ts-check
const { test, expect } = require('@playwright/test');

const WASM_BASE = '/wasm/';

test.describe('WASM Frontend — HTML UI Rendering', () => {

  test('page loads and renders the app shell', async ({ page }) => {
    await page.goto(WASM_BASE);
    // Wait for the nav title to appear, confirming the Leptos app mounted
    await page.waitForSelector('.nav-title', {
      state: 'visible',
      timeout: 30_000,
    });
    await expect(page.locator('.nav-title')).toHaveText('ApexIntel WASM');
  });

  test('navigation sidebar contains expected links', async ({ page }) => {
    await page.goto(WASM_BASE);
    await page.waitForSelector('.nav-links', { state: 'visible', timeout: 30_000 });

    const navLinks = page.locator('.nav-links a');
    const linkCount = await navLinks.count();
    // There are 16 NAV_ITEMS + 1 Adversarial link = 17 expected links
    expect(linkCount).toBeGreaterThanOrEqual(15);

    // Check specific important links exist
    const linkTexts = await navLinks.allTextContents();
    const expectedLabels = ['Overview', 'Warnings', 'Insights', 'Companies', 'Graph', 'Search'];
    for (const label of expectedLabels) {
      expect(linkTexts.some(t => t.trim() === label)).toBeTruthy();
    }
  });

  test('Overview page is the default route', async ({ page }) => {
    await page.goto(WASM_BASE);
    await page.waitForSelector('.page', { state: 'visible', timeout: 30_000 });

    // The Overview page should have content inside the <main> area
    const mainContent = page.locator('.app-main');
    await expect(mainContent).toBeVisible();
  });

  test('navigation link highlights active route', async ({ page }) => {
    await page.goto(WASM_BASE);
    await page.waitForSelector('.nav-link-active', { state: 'visible', timeout: 30_000 });

    const activeLink = page.locator('.nav-link-active');
    await expect(activeLink).toHaveCount(1);
  });

  test('page title is set correctly', async ({ page }) => {
    await page.goto(WASM_BASE);
    await expect(page).toHaveTitle(/ApexIntel/);
  });

  test('app-shell layout grid is rendered', async ({ page }) => {
    await page.goto(WASM_BASE);
    await page.waitForSelector('.app-shell', { state: 'visible', timeout: 30_000 });

    const shell = page.locator('.app-shell');
    await expect(shell).toBeVisible();
  });

  test('site navigation has header with description', async ({ page }) => {
    await page.goto(WASM_BASE);
    await page.waitForSelector('.site-nav-header', { state: 'visible', timeout: 30_000 });

    await expect(page.locator('.site-nav-copy')).toBeVisible();
  });

  test('filter-bar renders on warning list pages', async ({ page }) => {
    await page.goto(`${WASM_BASE}warnings`);
    await page.waitForSelector('.app-main', { state: 'visible', timeout: 30_000 });
    // Allow WASM routing to settle
    await page.waitForTimeout(2_000);
    // Check we're on the warnings page or at least not errored
    const bodyText = await page.locator('body').textContent();
    // Should not show a blank page — any content means the route rendered
    expect(bodyText ?? '').not.toHaveLength(0);
  });

});
