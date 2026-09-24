import { chromium } from 'playwright';
const exe = '/Users/sabelakhoua/Library/Caches/ms-playwright/chromium-1217/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing';
const browser = await chromium.launch({ executablePath: exe });
const p = await (await browser.newContext()).newPage();
p.on('console', m => { if (m.type() === 'error') console.log('CONSOLE-ERR:', m.text().slice(0, 180)); });
await p.goto('http://localhost:8844/', { waitUntil: 'load' });
const result = await p.evaluate(async () => {
  const deadline = Date.now() + 120000;
  while (Date.now() < deadline) {
    const state = document.querySelector('[data-kiwi-widget]')?.getAttribute('data-state');
    const token = document.getElementById('kiwi-token-input')?.value;
    if (token) return { state, tokenLen: token.length, preview: atob(token).slice(0, 60) };
    if (state === 'failed') return { state: 'failed' };
    await new Promise(r => setTimeout(r, 300));
  }
  return { timeout: true, state: document.querySelector('[data-kiwi-widget]')?.getAttribute('data-state') };
});
console.log(JSON.stringify(result, null, 1));
await browser.close();
