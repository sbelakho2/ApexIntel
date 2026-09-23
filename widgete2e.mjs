import { chromium } from 'playwright';
import { readFileSync } from 'fs';
const exe = '/Users/sabelakhoua/Library/Caches/ms-playwright/chromium-1217/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing';
const widgetHtml = readFileSync('/tmp/widget.html', 'utf8');
const browser = await chromium.launch({ executablePath: exe });
const p = await (await browser.newContext()).newPage();
p.on('console', m => { if (m.type() === 'error' || m.type() === 'warning') console.log('CONSOLE:', m.type(), m.text().slice(0, 150)); });
await p.goto('https://app.apexmail.ee/login/', { waitUntil: 'load', timeout: 30000 });
// Inject the widget into the page (same-origin so /api/kcaptcha works)
const result = await p.evaluate(async (html) => {
  const div = document.createElement('div');
  div.innerHTML = html;
  document.body.appendChild(div);
  // Wait for the widget to fetch + solve
  const deadline = Date.now() + 90000;
  while (Date.now() < deadline) {
    const state = document.querySelector('[data-kiwi-widget]')?.getAttribute('data-state');
    const token = document.getElementById('kiwi-token-input')?.value;
    if (token) return { state, tokenLen: token.length, tokenPrefix: token.slice(0, 24) };
    if (state === 'failed') return { state, token: '' };
    await new Promise(r => setTimeout(r, 300));
  }
  return { state: document.querySelector('[data-kiwi-widget]')?.getAttribute('data-state'), timeout: true };
}, widgetHtml);
console.log(JSON.stringify(result, null, 1));
await browser.close();
