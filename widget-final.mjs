import { chromium } from 'playwright';
const exe = '/Users/sabelakhoua/Library/Caches/ms-playwright/chromium-1217/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing';
const browser = await chromium.launch({ executablePath: exe });
const p = await (await browser.newContext()).newPage();
const violations = [];
p.on('console', m => { if (m.type() === 'error' && m.text().includes('Content Security')) violations.push(m.text().slice(0, 80)); });
p.on('pageerror', e => violations.push('PAGEERROR: ' + String(e).slice(0, 80)));
await p.goto('https://app.apexmail.ee/login/', { waitUntil: 'load', timeout: 30000 });
const r = await p.evaluate(async () => {
  const deadline = Date.now() + 30000;
  while (Date.now() < deadline) {
    const w = document.querySelector('[data-kiwi-widget]');
    const token = document.querySelector('[data-kiwi-token]')?.value;
    if (token) return { state: w?.getAttribute('data-state'), tokenLen: token.length, solveMs: atob(token).split('.')[2] };
    if (w?.getAttribute('data-state') === 'failed') return { state: 'failed' };
    await new Promise(r => setTimeout(r, 200));
  }
  return { state: document.querySelector('[data-kiwi-widget]')?.getAttribute('data-state'), timeout: true };
});
console.log(JSON.stringify({ r, cspViolations: violations.length, details: violations }, null, 1));
await browser.close();
