// @ts-check
const { test, expect } = require('@playwright/test');

const WASM_BASE = '/wasm/';

test.describe('WASM Frontend — Mobile Viewport Rendering', () => {

  test.use({
    viewport: { width: 375, height: 812 }, // iPhone X dimensions
  });

  test('mobile viewport shows topbar with navigation toggle', async ({ page }) => {
    await page.goto(WASM_BASE);
    await page.waitForSelector('.mobile-topbar', { state: 'visible', timeout: 30_000 });

    // Mobile topbar should be visible on small screens
    await expect(page.locator('.mobile-topbar')).toBeVisible();
  });

  test('mobile nav toggle button exists and is interactive', async ({ page }) => {
    await page.goto(WASM_BASE);
    await page.waitForSelector('.nav-toggle', { state: 'visible', timeout: 30_000 });

    const toggleButton = page.locator('.nav-toggle');
    await expect(toggleButton).toBeVisible();
    await expect(toggleButton).toHaveAttribute('aria-label', 'Toggle navigation');

    // Click to open nav
    await toggleButton.click();
    // The site-nav should slide in after clicking
    await page.waitForSelector('.site-nav-open', { state: 'visible', timeout: 5_000 });
  });

  test('mobile viewport sidebar is off-screen by default', async ({ page }) => {
    await page.goto(WASM_BASE);
    await page.waitForSelector('.site-nav', { state: 'visible', timeout: 30_000 });

    // Without the open class, the nav should not have site-nav-open
    const nav = page.locator('.site-nav');
    const hasOpenClass = await nav.evaluate(el => el.classList.contains('site-nav-open'));
    expect(hasOpenClass).toBe(false);
  });

  test('mobile viewport clicking toggle opens and closes nav', async ({ page }) => {
    await page.goto(WASM_BASE);
    await page.waitForSelector('.nav-toggle', { state: 'visible', timeout: 30_000 });

    const toggle = page.locator('.nav-toggle');
    const nav = page.locator('.site-nav');

    // Open nav
    await toggle.click();
    await page.waitForTimeout(500);
    let isOpen = await nav.evaluate(el => el.classList.contains('site-nav-open'));
    expect(isOpen).toBe(true);

    // Click backdrop to close
    const backdrop = page.locator('.nav-backdrop');
    await backdrop.click({ force: true });
    await page.waitForTimeout(500);
    isOpen = await nav.evaluate(el => el.classList.contains('site-nav-open'));
    expect(isOpen).toBe(false);
  });

  test('mobile viewport nav backdrop appears when nav is open', async ({ page }) => {
    await page.goto(WASM_BASE);
    await page.waitForSelector('.nav-toggle', { state: 'visible', timeout: 30_000 });

    // Open nav
    await page.locator('.nav-toggle').click();
    await page.waitForTimeout(500);

    // Backdrop should have open class
    const backdrop = page.locator('.nav-backdrop');
    const hasBackdropOpen = await backdrop.evaluate(el => el.classList.contains('nav-backdrop-open'));
    expect(hasBackdropOpen).toBe(true);
  });

  test('mobile viewport app shell collapses to single column', async ({ page }) => {
    await page.goto(WASM_BASE);
    await page.waitForSelector('.app-shell', { state: 'visible', timeout: 30_000 });

    // On mobile, app-shell should compute to a single grid track.
    const shell = page.locator('.app-shell');
    const gridTemplate = await shell.evaluate(el =>
      window.getComputedStyle(el).gridTemplateColumns
    );
    expect(gridTemplate.trim().split(/\s+/)).toHaveLength(1);
  });

  test('mobile viewport main area has reduced padding', async ({ page }) => {
    await page.goto(WASM_BASE);
    await page.waitForSelector('.app-main', { state: 'visible', timeout: 30_000 });

    const main = page.locator('.app-main');
    const padding = await main.evaluate(el =>
      window.getComputedStyle(el).padding
    );
    // On mobile, padding should be 1rem (16px) or less
    expect(padding).toContain('16px');
  });

  test('mobile viewport can navigate to different pages', async ({ page }) => {
    await page.goto(WASM_BASE);
    await page.waitForSelector('.nav-toggle', { state: 'visible', timeout: 30_000 });

    // Open nav
    await page.locator('.nav-toggle').click();
    await page.waitForTimeout(500);

    // Click on the Warnings link
    const warningsLink = page.locator('.nav-links a', { hasText: 'Warnings' });
    await warningsLink.click();
    await page.waitForTimeout(2_000);

    // Should have navigated to the warnings page
    const currentUrl = page.url();
    expect(currentUrl).toContain('warnings');
  });

});
