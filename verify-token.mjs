import { chromium } from 'playwright';
const exe = '/Users/sabelakhoua/Library/Caches/ms-playwright/chromium-1217/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing';
const browser = await chromium.launch({ executablePath: exe });
const p = await (await browser.newContext()).newPage();
// Fetch challenge, solve it with the wasm, then submit to the real login endpoint with a bogus password —
// we only need to prove the CAPTCHA itself verifies (i.e. response is 401 "invalid credentials",
// NOT 400 "CAPTCHA verification failed").
const result = await p.evaluate(async () => {
  const resp = await fetch('https://app.apexmail.ee/api/kcaptcha/challenge', { method: 'POST', headers: {'Content-Type':'application/json'}, body: JSON.stringify({scope:'login'}) });
  const data = await resp.json();
  // Solve sha256 20-bit via the embedded wasm
  const src = document.querySelector('#wasm-src')?.textContent || '';
  // load wasm from the widget asset inline
  return { algorithm: data.algorithm, targetBits: data.targetBits, mKib: data.mKib, minDurationMs: data.minDurationMs };
});
console.log(JSON.stringify(result, null, 1));
await browser.close();
