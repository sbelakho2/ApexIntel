import { chromium } from 'playwright';
const exe = '/Users/sabelakhoua/Library/Caches/ms-playwright/chromium-1217/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing';
const browser = await chromium.launch({ executablePath: exe });
const p = await (await browser.newContext()).newPage();
const logs = [];
p.on('console', m => logs.push(`[${m.type()}] ${m.text().slice(0, 150)}`));
p.on('pageerror', e => logs.push(`[PAGEERROR] ${String(e).slice(0, 150)}`));
await p.goto('https://app.apexmail.ee/login/', { waitUntil: 'load', timeout: 30000 });
await p.waitForTimeout(15000);
const r = await p.evaluate(async () => {
  const w = document.querySelector('[data-kiwi-widget]');
  const info = { state: w?.getAttribute('data-state'), label: w?.querySelector('[data-kiwi-label]')?.textContent };
  // Check if WASM loaded
  try {
    info.wasmAvailable = !!(window.__kiwiCaptchaWasm && await window.__kiwiCaptchaWasm.load());
  } catch (e) { info.wasmErr = String(e); }
  info.progressWidth = w?.querySelector('[data-kiwi-bar]')?.style.width;
  return info;
});
console.log(JSON.stringify({ r, logs }, null, 1));
await browser.close();
