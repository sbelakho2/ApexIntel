const { chromium } = require('playwright');
const path = require('path');
const fs = require('fs');

const BASE = 'https://starzerp.fi';
const OUT_DIR = path.join(__dirname, '..', 'screenshots');

const pages = [
  { path: '/login', name: 'login' },
  { path: '/', name: 'dashboard' },
  { path: '/insights', name: 'insights' },
  { path: '/warnings', name: 'warnings' },
  { path: '/companies', name: 'companies' },
  { path: '/persons', name: 'persons' },
  { path: '/competitors', name: 'competitors' },
  { path: '/graph', name: 'graph' },
  { path: '/recipes', name: 'recipes' },
  { path: '/memos', name: 'memos' },
  { path: '/search', name: 'search' },
  { path: '/security', name: 'security' },
  { path: '/trends', name: 'trends' },
  { path: '/executive', name: 'executive' },
  { path: '/battlecards', name: 'battlecards' },
];

(async () => {
  fs.mkdirSync(OUT_DIR, { recursive: true });

  // ── Desktop screenshots (1440x900, light mode) ──
  console.log('=== DESKTOP SCREENSHOTS (light mode) ===');
  const desktopBrowser = await chromium.launch({ headless: true });
  const desktopCtx = await desktopBrowser.newContext({
    viewport: { width: 1440, height: 900 },
    ignoreHTTPSErrors: true,
    colorScheme: 'light',
  });
  const desktopPage = await desktopCtx.newPage();

  // Login
  console.log('Logging in...');
  await desktopPage.goto(`${BASE}/login`, { waitUntil: 'networkidle', timeout: 15000 });
  await desktopPage.fill('input[name="username"]', 'aaron');
  await desktopPage.fill('input[name="password"]', 'adminpassword');
  await desktopPage.click('button[type="submit"]');
  await desktopPage.waitForTimeout(3000);
  console.log('Login attempted');

  for (const { path: p, name } of pages) {
    try {
      console.log(`Desktop: ${p}`);
      await desktopPage.goto(`${BASE}${p}`, { waitUntil: 'networkidle', timeout: 15000 });
      // Force light theme
      await desktopPage.evaluate(() => {
        localStorage.setItem('apex-theme', 'light');
        document.documentElement.classList.remove('dark');
        document.documentElement.removeAttribute('data-theme');
      });
      await desktopPage.goto(`${BASE}${p}`, { waitUntil: 'networkidle', timeout: 15000 });
      await desktopPage.waitForTimeout(3000);
      if (p === '/graph') await desktopPage.waitForTimeout(10000);
      await desktopPage.screenshot({
        path: path.join(OUT_DIR, `desktop_${name}.png`),
        fullPage: false,
      });
      console.log(`  Saved: desktop_${name}.png`);
    } catch (err) {
      console.log(`  FAIL: ${err.message}`);
      // Still try to capture what we can
      try {
        await desktopPage.screenshot({
          path: path.join(OUT_DIR, `desktop_${name}_error.png`),
          fullPage: false,
        });
      } catch (_) {}
    }
  }

  await desktopBrowser.close();

  // ── Mobile screenshots (390x844, light mode) ──
  console.log('\n=== MOBILE SCREENSHOTS (390x844) ===');
  const mobileBrowser = await chromium.launch({ headless: true });
  const mobileCtx = await mobileBrowser.newContext({
    viewport: { width: 390, height: 844 },
    ignoreHTTPSErrors: true,
    colorScheme: 'light',
    isMobile: true,
    hasTouch: true,
  });
  const mobilePage = await mobileCtx.newPage();

  // Login
  console.log('Mobile login...');
  await mobilePage.goto(`${BASE}/login`, { waitUntil: 'networkidle', timeout: 15000 });
  await mobilePage.fill('input[name="username"]', 'aaron');
  await mobilePage.fill('input[name="password"]', 'adminpassword');
  await mobilePage.click('button[type="submit"]');
  await mobilePage.waitForTimeout(3000);

  for (const { path: p, name } of pages.slice(1)) {
    try {
      console.log(`Mobile: ${p}`);
      await mobilePage.goto(`${BASE}${p}`, { waitUntil: 'networkidle', timeout: 15000 });
      await mobilePage.evaluate(() => {
        localStorage.setItem('apex-theme', 'light');
        document.documentElement.classList.remove('dark');
      });
      await mobilePage.goto(`${BASE}${p}`, { waitUntil: 'networkidle', timeout: 15000 });
      await mobilePage.waitForTimeout(3000);
      if (p === '/graph') await mobilePage.waitForTimeout(10000);
      await mobilePage.screenshot({
        path: path.join(OUT_DIR, `mobile_${name}.png`),
        fullPage: false,
      });
      console.log(`  Saved: mobile_${name}.png`);
    } catch (err) {
      console.log(`  FAIL: ${err.message}`);
    }
  }

  await mobileBrowser.close();
  console.log('\nDone! All screenshots captured.');
})();
