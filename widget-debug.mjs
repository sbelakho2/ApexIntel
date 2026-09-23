import { chromium } from 'playwright';
const exe = '/Users/sabelakhoua/Library/Caches/ms-playwright/chromium-1217/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing';
const fs = await import('fs');
const driver = fs.readFileSync('/Users/sabelakhoua/IdeaProjects/ApexMail/packages/kiwicaptcha-wasm/assets/widget-driver.js', 'utf8');
const wasmEmbed = fs.readFileSync('/Users/sabelakhoua/IdeaProjects/ApexMail/packages/kiwicaptcha-wasm/assets/kiwicaptcha-wasm.js', 'utf8');
const browser = await chromium.launch({ executablePath: exe });
const p = await (await browser.newContext()).newPage();
const logs = [];
p.on('console', m => logs.push(`[${m.type()}] ${m.text().slice(0, 150)}`));
p.on('pageerror', e => logs.push(`[PAGEERROR] ${String(e).slice(0, 200)}`));
await p.goto('https://app.apexmail.ee/login/', { waitUntil: 'load' });
const r = await p.evaluate(async ({ driver, wasmEmbed }) => {
  // Check exports of the loaded wasm directly
  const w = await window.__kiwiCaptchaWasm.load();
  const exports = Object.keys(w).filter(k => /alloc|dealloc|solve|wbindgen/.test(k));
  return { exports };
}, { driver, wasmEmbed });
console.log(JSON.stringify({ r, logs }, null, 1));
await browser.close();
