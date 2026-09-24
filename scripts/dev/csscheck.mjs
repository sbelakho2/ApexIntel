import { chromium } from 'playwright';
const exe = '/Users/sabelakhoua/Library/Caches/ms-playwright/chromium-1217/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing';
const browser = await chromium.launch({ executablePath: exe });
const ctx = await browser.newContext();
const p = await ctx.newPage();
const errors = [];
p.on('console', m => { if (m.type() === 'error') errors.push(m.text().slice(0, 200)); });
p.on('pageerror', e => errors.push('PAGEERROR: ' + e.message.slice(0, 200)));
p.on('requestfailed', r => errors.push('REQFAIL: ' + r.url().slice(0, 150)));
for (const url of ['https://apexmail.ee/solutions/enterprise/', 'https://apexmail.ee/']) {
  const resp = await p.goto(url, { waitUntil: 'load', timeout: 30000 });
  await p.waitForTimeout(1500);
  const r = await p.evaluate(() => {
    const h1 = document.querySelector('h1');
    const hero = document.querySelector('section');
    const cs1 = h1 ? getComputedStyle(h1) : null;
    const body = getComputedStyle(document.body);
    return {
      url: location.pathname,
      h1: h1 ? h1.textContent.trim().slice(0, 50) : null,
      h1Size: cs1 ? cs1.fontSize : null,
      h1Weight: cs1 ? cs1.fontWeight : null,
      h1Color: cs1 ? cs1.color : null,
      bodyFont: body.fontFamily,
      bodyBg: body.backgroundColor,
      sheets: Array.from(document.styleSheets).map(s => { try { return (s.href || 'inline').split('/').pop(); } catch (e) { return 'ERR'; } }),
      heroBg: hero ? getComputedStyle(hero).backgroundColor : null,
    };
  });
  console.log(url, JSON.stringify(r));
}
console.log('ERRORS:', JSON.stringify(errors));
await browser.close();
