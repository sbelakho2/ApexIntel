import { chromium } from 'playwright';
const exe = '/Users/sabelakhoua/Library/Caches/ms-playwright/chromium-1217/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing';
const browser = await chromium.launch({ executablePath: exe });
const p = await (await browser.newContext()).newPage();
const logs = [];
p.on('console', m => logs.push(`[${m.type()}] ${m.text().slice(0, 200)}`));
p.on('pageerror', e => logs.push(`[PAGEERROR] ${String(e).slice(0, 200)}`));
await p.goto('https://app.apexmail.ee/login/', { waitUntil: 'load', timeout: 30000 });
await p.waitForTimeout(5000);
const r = await p.evaluate(() => {
  const w = document.querySelector('[data-kiwi-widget]');
  return {
    state: w?.getAttribute('data-state'),
    label: w?.querySelector('[data-kiwi-label]')?.textContent,
    hasToken: !!document.querySelector('[data-kiwi-token]')?.value,
    scripts: Array.from(document.querySelectorAll('script')).map(s => s.src || 'inline:' + s.textContent.slice(0, 40)),
    styles: Array.from(document.querySelectorAll('style')).map(s => s.textContent.slice(0, 40)),
  };
});
console.log(JSON.stringify({ page: r, logs }, null, 1));
await browser.close();
