import { chromium } from 'playwright';
const exe = '/Users/sabelakhoua/Library/Caches/ms-playwright/chromium-1217/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing';
const browser = await chromium.launch({ executablePath: exe });
const p = await (await browser.newContext()).newPage();
await p.goto('http://localhost:8846/', { waitUntil: 'load' });
const deadline = Date.now() + 60000;
let result = null;
while (Date.now() < deadline) {
  result = await p.evaluate(() => window.__result || null);
  if (result) break;
  await new Promise(r => setTimeout(r, 300));
}
console.log(result || JSON.stringify({timeout: true, state: await p.evaluate(() => document.querySelector('[data-kiwi-widget]')?.getAttribute('data-state'))}));
await browser.close();
