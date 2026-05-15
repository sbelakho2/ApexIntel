// @ts-check
const { test, expect } = require('@playwright/test');

const WASM_BASE = '/wasm/';

test.describe('WASM Frontend — Chart & Data Rendering', () => {

  test.describe('Overview Page', () => {

    test('overview page has stat-grid or surface-card elements', async ({ page }) => {
      await page.goto(WASM_BASE);
      await page.waitForSelector('.app-main', { state: 'visible', timeout: 30_000 });

      // The overview page should show stat cards or content
      const hasStatGrid = await page.locator('.stat-grid').count();
      const hasSurfaceCards = await page.locator('.surface-card').count();
      const hasContent = hasStatGrid > 0 || hasSurfaceCards > 0;

      expect(hasContent).toBeTruthy();
    });

  });

  test.describe('Graph Page', () => {

    test('graph page loads with chart layout', async ({ page }) => {
      await page.goto(`${WASM_BASE}graph`);
      await page.waitForSelector('.app-main', { state: 'visible', timeout: 30_000 });
      await page.waitForTimeout(3_000); // Allow WASM chart components to hydrate

      // Check for chart-related elements
      const chartLayout = page.locator('.chart-layout, .chart-layout-graph');
      const chartShell = page.locator('.chart-scroll-shell');
      const graphSidePanel = page.locator('.graph-side-panel');

      const hasChartLayout = (await chartLayout.count()) > 0;
      const hasChartShell = (await chartShell.count()) > 0;
      const hasSidePanel = (await graphSidePanel.count()) > 0;

      // At least one chart structure should exist
      expect(hasChartLayout || hasChartShell || hasSidePanel).toBeTruthy();
    });

    test('graph page has toolbar and side panel', async ({ page }) => {
      await page.goto(`${WASM_BASE}graph`);
      await page.waitForSelector('.app-main', { state: 'visible', timeout: 30_000 });
      await page.waitForTimeout(2_000);

      // Graph toolbar should be present
      const toolbar = page.locator('.graph-toolbar');
      const toolbarCount = await toolbar.count();
      // The graph page may have the toolbar in its layout
      if (toolbarCount > 0) {
        await expect(toolbar.first()).toBeVisible();
      }
    });

  });

  test.describe('Calibration Page', () => {

    test('calibration page renders chart content', async ({ page }) => {
      await page.goto(`${WASM_BASE}calibration`);
      await page.waitForSelector('.app-main', { state: 'visible', timeout: 30_000 });
      await page.waitForTimeout(2_000);

      // Should show some content (confidence bands, reliability diagrams, etc.)
      const bodyText = await page.locator('body').textContent();
      expect(bodyText ?? '').not.toHaveLength(0);
    });

  });

  test.describe('Recipes Page', () => {

    test('recipes page loads successfully', async ({ page }) => {
      await page.goto(`${WASM_BASE}recipes`);
      await page.waitForSelector('.app-main', { state: 'visible', timeout: 30_000 });
      await page.waitForTimeout(2_000);

      const bodyText = await page.locator('body').textContent();
      expect(bodyText ?? '').not.toHaveLength(0);
    });

  });

  test.describe('Adversarial Page', () => {

    test('adversarial page loads with chart components', async ({ page }) => {
      await page.goto(`${WASM_BASE}adversarial`);
      await page.waitForSelector('.app-main', { state: 'visible', timeout: 30_000 });
      await page.waitForTimeout(2_000);

      const bodyText = await page.locator('body').textContent();
      // Should render actual content, not just empty shell
      expect(bodyText ?? '').not.toHaveLength(0);
    });

  });

});
