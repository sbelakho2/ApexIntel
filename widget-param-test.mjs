// Verify the NEW parameterized widget DOM (endpoint/scope on container, token
// as first child of container) works with the driver end-to-end.
import { chromium } from 'playwright';
const exe = '/Users/sabelakhoua/Library/Caches/ms-playwright/chromium-1217/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing';
const fs = await import('fs');
const driver = fs.readFileSync('/Users/sabelakhoua/IdeaProjects/ApexMail/packages/kiwicaptcha-wasm/assets/widget-driver.js', 'utf8');
const wasmEmbed = fs.readFileSync('/Users/sabelakhoua/IdeaProjects/ApexMail/packages/kiwicaptcha-wasm/assets/kiwicaptcha-wasm.js', 'utf8');
const css = fs.readFileSync('/Users/sabelakhoua/IdeaProjects/ApexMail/packages/kiwicaptcha-wasm/assets/widget.css', 'utf8');
const browser = await chromium.launch({ executablePath: exe });
const p = await (await browser.newContext()).newPage();
const logs = [];
p.on('console', m => { if (m.type() === 'error') logs.push(m.text().slice(0, 100)); });
await p.goto('https://app.apexmail.ee/login/', { waitUntil: 'load' });
const r = await p.evaluate(async ({ driver, wasmEmbed, css }) => {
  // Build the NEW layout exactly as widget.rs emits it
  const wrap = document.createElement('div');
  wrap.innerHTML = `<style>${css}</style>
  <div class="kiwi-container" id="kiwicaptcha-root" data-kiwi-endpoint="/api/kcaptcha/challenge" data-kiwi-scope="login">
    <input type="hidden" name="kiwi__token" data-kiwi-token value="" />
    <div class="kiwi-widget" data-kiwi-widget data-state="idle" role="status" aria-live="polite">
      <div class="kiwi-icon-wrapper"><svg></svg><div class="kiwi-glow"></div></div>
      <div class="kiwi-main">
        <div class="kiwi-top"><span class="kiwi-label" data-kiwi-label>Security Check</span><span class="kiwi-badge" data-kiwi-badge>Idle</span></div>
        <div class="kiwi-track"><div class="kiwi-bar" data-kiwi-bar></div></div>
        <div class="kiwi-bottom"><p class="kiwi-info" data-kiwi-info>Protected</p><span class="kiwi-timer" data-kiwi-timer></span></div>
      </div>
    </div>
  </div>`;
  document.body.appendChild(wrap);
  const s1 = document.createElement('script'); s1.textContent = wasmEmbed; document.body.appendChild(s1);
  const s2 = document.createElement('script'); s2.textContent = driver; document.body.appendChild(s2);
  const deadline = Date.now() + 30000;
  while (Date.now() < deadline) {
    const token = document.querySelector('[data-kiwi-token]')?.value;
    const state = document.querySelector('[data-kiwi-widget]')?.getAttribute('data-state');
    if (token) { const parts = atob(token).split('.'); return { state, counter: parts[1], durationMs: parts[2], scope: parts[0] ? 'token' : 'none' }; }
    if (state === 'failed') return { state: 'failed' };
    await new Promise(r => setTimeout(r, 200));
  }
  return { state: document.querySelector('[data-kiwi-widget]')?.getAttribute('data-state'), timeout: true };
}, { driver, wasmEmbed, css });
console.log(JSON.stringify({ r, logs }, null, 1));
await browser.close();
