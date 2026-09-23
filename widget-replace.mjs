import { chromium } from 'playwright';
const exe = '/Users/sabelakhoua/Library/Caches/ms-playwright/chromium-1217/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing';
const fs = await import('fs');
const driver = fs.readFileSync('/Users/sabelakhoua/IdeaProjects/ApexMail/packages/kiwicaptcha-wasm/assets/widget-driver.js', 'utf8');
const browser = await chromium.launch({ executablePath: exe });
const p = await (await browser.newContext()).newPage();
const logs = [];
p.on('console', m => logs.push(`[${m.type()}] ${m.text().slice(0, 150)}`));
p.on('pageerror', e => logs.push(`[PAGEERROR] ${String(e).slice(0, 200)}`));
await p.goto('https://app.apexmail.ee/login/', { waitUntil: 'load' });
// Reset the real widget's token + re-run its init with the NEW driver logic
const r = await p.evaluate(async ({ driver }) => {
  const w = document.querySelector('[data-kiwi-widget]');
  // clear stale state
  const token = document.querySelector('[data-kiwi-token]');
  token.value = '';
  w.setAttribute('data-state', 'idle');
  delete w.dataset.kiwiStarted;
  // run the fixed driver (it re-scans and initializes)
  eval(driver);
  const deadline = Date.now() + 30000;
  while (Date.now() < deadline) {
    const t = document.querySelector('[data-kiwi-token]')?.value;
    if (t) { const parts = atob(t).split('.'); return { state: w.getAttribute('data-state'), counter: parts[1], durationMs: parts[2] }; }
    if (w.getAttribute('data-state') === 'failed') return { state: 'failed' };
    await new Promise(r => setTimeout(r, 200));
  }
  return { state: w.getAttribute('data-state'), timeout: true };
}, { driver });
console.log(JSON.stringify({ r, logs }, null, 1));
await browser.close();
