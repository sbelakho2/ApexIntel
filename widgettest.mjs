import { chromium } from 'playwright';
const exe = '/Users/sabelakhoua/Library/Caches/ms-playwright/chromium-1217/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing';
const browser = await chromium.launch({ executablePath: exe });
const p = await (await browser.newContext()).newPage();
const result = await p.evaluate(async () => {
  // Fetch a real challenge from the deployed API
  const resp = await fetch('https://app.apexmail.ee/api/kcaptcha/challenge', {
    method: 'POST', headers: {'Content-Type': 'application/json'},
    body: JSON.stringify({scope: 'login'})
  });
  const data = await resp.json();
  // Extract the widget script from the login page to test in context
  const page = await (await fetch('https://app.apexmail.ee/login/')).text();
  return { algorithm: data.algorithm, mKib: data.mKib, targetBits: data.targetBits, hasWidget: page.includes('kiwicaptcha-widget') };
}, );
console.log(JSON.stringify(result, null, 1));
await browser.close();
