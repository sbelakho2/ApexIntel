import { chromium } from 'playwright';
const exe = '/Users/sabelakhoua/Library/Caches/ms-playwright/chromium-1217/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing';
const browser = await chromium.launch({ executablePath: exe });
const p = await (await browser.newContext()).newPage();
const violations = [];
p.on('console', m => { if (m.type() === 'error' && m.text().includes('Content Security Policy')) violations.push(m.text().slice(0, 120)); });
const reqs = [];
p.on('request', r => { const u = r.url(); if (/fonts\.google|cdn\.|gstatic|googleapis/i.test(u)) reqs.push(u); });

await p.goto('https://app.apexmail.ee/login/', { waitUntil: 'load' });

const r = await p.evaluate(async () => {
  const out = { state: null, token: null, endpoint: null, scope: null, h1: false, logo: false, newStyle: false, thirdParty: null };
  const deadline = Date.now() + 30000;
  while (Date.now() < deadline) {
    const w = document.querySelector('[data-kiwi-widget]');
    if (w) {
      out.state = w.getAttribute('data-state');
      const c = document.querySelector('.kiwi-container');
      if (c) { out.endpoint = c.getAttribute('data-kiwi-endpoint'); out.scope = c.getAttribute('data-kiwi-scope'); }
      out.h1 = !!document.querySelector('main h1');
      out.logo = !!document.querySelector('main .text-surface-950');
      out.newStyle = getComputedStyle(document.querySelector('.kiwi-widget') || document.body).backgroundColor !== 'rgba(0, 0, 0, 0)';
      const t = document.querySelector('[data-kiwi-token]');
      if (t && t.value) { out.token = t.value.slice(0, 12) + '...'; return out; }
    }
    await new Promise(r2 => setTimeout(r2, 200));
  }
  return out;
});
console.log(JSON.stringify({ r, violations, thirdPartyFetches: reqs }, null, 1));
await browser.close();
