import { chromium } from 'playwright';
const exe = '/Users/sabelakhoua/Library/Caches/ms-playwright/chromium-1217/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing';
const browser = await chromium.launch({ executablePath: exe, args: ['--disable-blink-features=AutomationControlled'] });
const ctx = await browser.newContext();
// Mask automation signals like a real browser
await ctx.addInitScript(() => {
  Object.defineProperty(navigator, 'webdriver', { get: () => false });
  Object.defineProperty(navigator, 'plugins', { get: () => [1,2,3,4,5] });
  window.chrome = { runtime: {} };
});
const p = await ctx.newPage();
await p.goto('https://app.apexmail.ee/login/', { waitUntil: 'load' });
// Simulate human interaction so telemetry shows discrete events
await p.mouse.move(300, 300);
await p.mouse.move(400, 350);
await p.keyboard.press('KeyA');
await p.keyboard.press('KeyB');
const result = await p.evaluate(async () => {
  const deadline = Date.now() + 60000;
  let token = null;
  while (Date.now() < deadline) {
    token = document.getElementById('kiwi-token-input')?.value;
    if (token) break;
    await new Promise(r => setTimeout(r, 100));
  }
  if (!token) return { error: 'no token' };
  const parts = atob(token).split('.');
  const csrf = document.querySelector('input[name="_csrf"]')?.value;
  const loginResp = await fetch('/v1/auth/login', {
    method: 'POST',
    headers: {'Content-Type':'application/json', 'X-CSRF-Token': csrf},
    body: JSON.stringify({email:'sabelakho@apexmail.ee', password:'definitely-wrong-password', kiwi__token: token})
  });
  const body = await loginResp.text();
  return { counter: parts[1], durationMs: parts[2], loginStatus: loginResp.status, loginBody: body.slice(0, 200) };
});
console.log(JSON.stringify(result, null, 1));
await browser.close();
