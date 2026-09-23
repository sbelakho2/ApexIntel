import { chromium } from 'playwright';
const exe = '/Users/sabelakhoua/Library/Caches/ms-playwright/chromium-1217/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing';
const browser = await chromium.launch({ executablePath: exe });
const p = await (await browser.newContext()).newPage();
p.on('console', m => { if (m.type() === 'error' || m.type() === 'warning') console.log('CONSOLE:', m.text().slice(0, 120)); });
await p.goto('https://app.apexmail.ee/login/', { waitUntil: 'load' });
// The real page renders the widget — wait for it to complete
const result = await p.evaluate(async () => {
  const deadline = Date.now() + 90000;
  while (Date.now() < deadline) {
    const state = document.querySelector('[data-kiwi-widget]')?.getAttribute('data-state');
    const token = document.getElementById('kiwi-token-input')?.value;
    if (token) return { state, tokenLen: token.length, preview: atob(token).slice(0, 40) };
    if (state === 'failed') return { state };
    await new Promise(r => setTimeout(r, 300));
  }
  return { timeout: true, state: document.querySelector('[data-kiwi-widget]')?.getAttribute('data-state') };
});
console.log(JSON.stringify(result, null, 1));
await browser.close();
