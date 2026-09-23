import { chromium } from 'playwright';
const exe = '/Users/sabelakhoua/Library/Caches/ms-playwright/chromium-1217/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing';
const browser = await chromium.launch({ executablePath: exe });
const p = await (await browser.newContext()).newPage();
const logs = [];
p.on('console', m => { if (m.type() === 'warning' && m.text().includes('WASM')) logs.push(m.text().slice(0, 100)); });
await p.goto('https://app.apexmail.ee/login/', { waitUntil: 'load' });
const result = await p.evaluate(async () => {
  // Wait for the real widget to complete
  const deadline = Date.now() + 60000;
  let token = null, state = null, solveTime = null;
  while (Date.now() < deadline) {
    token = document.getElementById('kiwi-token-input')?.value;
    state = document.querySelector('[data-kiwi-widget]')?.getAttribute('data-state');
    if (token) break;
    await new Promise(r => setTimeout(r, 100));
  }
  if (!token) return { state, error: 'no token' };
  const parts = atob(token).split('.');
  // Now submit to real login with correct CSRF from the page
  const csrfInput = document.querySelector('input[name="_csrf"]');
  const csrf = csrfInput ? csrfInput.value : null;
  const loginResp = await fetch('/v1/auth/login', {
    method: 'POST',
    headers: {'Content-Type':'application/json', 'X-CSRF-Token': csrf},
    body: JSON.stringify({email:'sabelakho@apexmail.ee', password:'definitely-wrong-password', kiwi__token: token})
  });
  const body = await loginResp.text();
  return { state, counter: parts[1], durationMs: parts[2], csrfFound: !!csrf, loginStatus: loginResp.status, loginBody: body.slice(0, 200) };
});
console.log(JSON.stringify({ ...result, wasmFallbackWarnings: logs }, null, 1));
await browser.close();
