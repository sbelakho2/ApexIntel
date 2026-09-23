import { chromium } from 'playwright';
const exe = '/Users/sabelakhoua/Library/Caches/ms-playwright/chromium-1217/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing';
const browser = await chromium.launch({ executablePath: exe });
const p = await (await browser.newContext()).newPage();
await p.goto('http://localhost:8845/', { waitUntil: 'load' });
const result = await p.evaluate(async () => {
  const deadline = Date.now() + 60000;
  while (Date.now() < deadline) {
    const token = document.getElementById('kiwi-token-input')?.value;
    const state = document.querySelector('[data-kiwi-widget]')?.getAttribute('data-state');
    if (token) {
      // Verify the token shape: nonce.counter.duration.telemetry (base64)
      const plain = atob(token);
      const parts = plain.split('.');
      return { state, counter: parts[1], durationMs: parts[2], hasTelemetry: parts.length === 4 };
    }
    if (state === 'failed') return { state: 'failed' };
    await new Promise(r => setTimeout(r, 200));
  }
  return { timeout: true };
});
console.log(JSON.stringify(result, null, 1));
await browser.close();
