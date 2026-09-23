import { chromium } from 'playwright';
const exe = '/Users/sabelakhoua/Library/Caches/ms-playwright/chromium-1217/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing';
const fs = await import('fs');
const asset = fs.readFileSync('/Users/sabelakhoua/IdeaProjects/ApexMail/packages/kiwicaptcha-wasm/assets/kiwicaptcha-wasm.js', 'utf8');
const browser = await chromium.launch({ executablePath: exe });
const p = await (await browser.newContext()).newPage();
const result = await p.evaluate(async (src) => {
  try {
    eval(src);
    const w = await window.__kiwiCaptchaWasm.load();
    return { loaded: true, exports: Object.keys(w).slice(0, 10) };
  } catch (e) {
    return { loaded: false, error: String(e), stack: String(e.stack || '').slice(0, 400) };
  }
}, asset);
console.log(JSON.stringify(result, null, 1));
await browser.close();
