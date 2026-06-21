const { chromium } = require('playwright');
const path = require('path');
const fs = require('fs');

const BASE = 'http://localhost:9095';
const OUT_DIR = path.join(__dirname, '..', 'screenshots');

(async () => {
  fs.mkdirSync(OUT_DIR, { recursive: true });

  const browser = await chromium.launch({ headless: true });
  const context = await browser.newContext({
    viewport: { width: 390, height: 844 },
    ignoreHTTPSErrors: true,
    colorScheme: 'dark',
    deviceScaleFactor: 2,
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

  // Force dark theme
  await page.evaluate(() => localStorage.setItem('apex-theme', 'dark'));
  await page.goto(`${BASE}/`, { waitUntil: 'networkidle', timeout: 15000 });
  await page.waitForTimeout(1000);

  // Verify burger button
  const btnInfo = await page.evaluate(() => {
    const btn = document.getElementById('mobile-nav-toggle');
    if (!btn) return { error: 'mobile-nav-toggle not found' };
    const rect = btn.getBoundingClientRect();
    return {
      exists: true,
      rect: { x: rect.x, y: rect.y, w: rect.width, h: rect.height },
      visible: btn.offsetParent !== null,
      ariaExpanded: btn.getAttribute('aria-expanded'),
    };
  });
  console.log('Burger button:', JSON.stringify(btnInfo, null, 2));

  // Click burger button to open sidebar
  await page.click('#mobile-nav-toggle');
  await page.waitForTimeout(600); // wait for 180ms transition

  // Capture sidebar state
  const state = await page.evaluate(() => {
    const nav = document.getElementById('mobile-nav');
    const overlay = document.getElementById('mobile-nav-overlay');
    if (!nav || !overlay) return { error: 'nav or overlay missing' };
    const ns = getComputedStyle(nav);
    const os = getComputedStyle(overlay);
    return {
      isOpen: nav.classList.contains('is-open'),
      hidden: nav.hasAttribute('hidden'),
      transform: ns.transform,
      bgColor: ns.backgroundColor,
      boxShadow: ns.boxShadow,
      borderRightColor: ns.borderRightColor,
      borderRightWidth: ns.borderRightWidth,
      overlayOpacity: os.opacity,
      overlayBg: os.backgroundColor,
      overlayPointerEvents: os.pointerEvents,
    };
  });
  console.log('Sidebar state:', JSON.stringify(state, null, 2));

  // Take a screenshot of the whole viewport with sidebar open
  await page.screenshot({
    path: path.join(OUT_DIR, 'sidebar-open.png'),
    fullPage: false,
  });
  console.log('Saved: sidebar-open.png (with sidebar open)');

  // Also test the sidebar on a few other pages
  for (const p of ['/warnings', '/insights', '/companies']) {
    await page.goto(`${BASE}${p}`, { waitUntil: 'networkidle', timeout: 15000 });
    await page.waitForTimeout(1000);
    await page.evaluate(() => localStorage.setItem('apex-theme', 'dark'));
    await page.waitForTimeout(500);
    // Open sidebar
    await page.click('#mobile-nav-toggle');
    await page.waitForTimeout(600);
    const safeName = p.replace(/\//g, '_').replace(/^_/, '');
    await page.screenshot({
      path: path.join(OUT_DIR, `sidebar-open-${safeName}.png`),
      fullPage: false,
    });
    console.log(`Saved: sidebar-open-${safeName}.png`);
    // Close sidebar for next
    await page.evaluate(() => {
      const closeBtn = document.getElementById('mobile-nav-close');
      if (closeBtn) closeBtn.click();
    });
    await page.waitForTimeout(600);
  }

  await browser.close();
  console.log('Done!');
})();
