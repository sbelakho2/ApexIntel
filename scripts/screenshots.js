const { chromium } = require('playwright');
const path = require('path');
const fs = require('fs');

const BASE = 'http://127.0.0.1:9095';
const OUT_DIR = path.join(__dirname, '..', 'screenshots');

const pages = [
  '/login',
  '/',
  '/executive',
  '/trends',
  '/warnings',
  '/insights',
  '/triage',
  '/workspaces',
  '/queue',
  '/activity',
  '/memos',
  '/notifications',
  '/companies',
  '/persons',
  '/competitors',
  '/battlecards',
  '/security',
  '/supplier-risk',
  '/pipeline',
  '/graph',
  '/recipes',
  '/search',
  '/settings',
  '/settings/alerts',
];

(async () => {
  fs.mkdirSync(OUT_DIR, { recursive: true });
  
  const browser = await chromium.launch({ headless: true });
  const context = await browser.newContext({
    viewport: { width: 1440, height: 900 }, // Desktop viewport
    ignoreHTTPSErrors: true,
    colorScheme: 'dark',
  });
  const page = await context.newPage();

  // Step 1: Login
  console.log('Logging in...');
  await page.goto(`${BASE}/login`, { waitUntil: 'networkidle' });
  await page.fill('input[name="username"]', 'admin');
  await page.fill('input[name="password"]', 'adminpassword');
  await page.click('button[type="submit"]');
  await page.waitForURL('**/');
  await page.waitForTimeout(1000);
  console.log('Login successful');

  // Step 2: Force dark theme via localStorage before each capture
  // This ensures the apex-theme key is set to 'dark' so base.html's
  // inline script resolves the dark class on <html>.
  await page.evaluate(() => {
    localStorage.setItem('apex-theme', 'dark');
  });
  // Reload after setting localStorage so the page picks up the dark theme
  await page.goto(`${BASE}/`, { waitUntil: 'networkidle', timeout: 15000 });
  await page.waitForTimeout(500);

  // Step 3: Capture all pages as authenticated user, re-applying dark theme
  for (const p of pages) {
    try {
      console.log(`Screenshot: ${p}`);
      const safeName = p === '/' ? 'dashboard' : p.replace(/\//g, '_').replace(/^_/, '');
      await page.goto(`${BASE}${p}`, { waitUntil: 'networkidle', timeout: 15000 });
      // Re-apply dark theme after navigation in case page load resets localStorage read
      await page.evaluate(() => {
        localStorage.setItem('apex-theme', 'dark');
      });
      // Wait for data to fully render — longer timeout for charts/tables on mobile
      await page.waitForTimeout(5000);
      // Wait for actual content elements (cards, tables, data containers) to appear
      await page.waitForSelector('.apex-card, table, .data-table, main > *, .graph-kpi-grid', { timeout: 5000 }).catch(() => {});
      // Also wait for main element to exist
      try {
        await page.waitForSelector('main', { timeout: 3000 });
      } catch (_) {
        // main should always exist; if not, proceed anyway
      }
      // For the graph page (D3.js force-directed graph), wait extra time
      // for the SVG canvas to fully render with nodes and links
      if (p === '/graph') {
        console.log('  (graph page: waiting extra for D3.js force layout to settle...)');
        await page.waitForTimeout(12000);
        await page.waitForSelector('#graph-container svg, #graph-container canvas', { timeout: 10000 }).catch(() => {});
        await page.waitForTimeout(3000);
      }
      await page.screenshot({
        path: path.join(OUT_DIR, `${safeName}.png`),
        fullPage: false
      });
      console.log(`  Saved: ${safeName}.png`);
    } catch (err) {
      console.log(`  FAIL: ${err.message}`);
    }
  }

  await browser.close();
  console.log('Done! All screenshots captured.');
})();