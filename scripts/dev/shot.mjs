import { chromium } from 'playwright';
const exe = '/Users/sabelakhoua/Library/Caches/ms-playwright/chromium-1217/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing';
const browser = await chromium.launch({ executablePath: exe });
const ctx = await browser.newContext({ viewport: { width: 1280, height: 900 } });
const p = await ctx.newPage();
for (const [i, url] of ['https://apexmail.ee/solutions/enterprise/', 'https://apexmail.ee/', 'https://apexmail.ee/pricing/'].entries()) {
  await p.goto(url, { waitUntil: 'networkidle', timeout: 30000 });
  await p.waitForTimeout(1000);
  await p.screenshot({ path: `/tmp/shot-${i}.png` });
  const info = await p.evaluate(() => ({
    url: location.pathname,
    h1: document.querySelector('h1')?.textContent?.trim().slice(0, 40),
    h1Size: getComputedStyle(document.querySelector('h1')).fontSize,
    mainLinks: document.querySelectorAll('main a, main p, main h2, main table').length,
    hasTable: !!document.querySelector('table'),
    tableWidth: document.querySelector('table') ? document.querySelector('table').offsetWidth : null,
    viewport: window.innerWidth,
    bodyScrollW: document.body.scrollWidth,
  }));
  console.log(JSON.stringify(info));
}
await browser.close();
