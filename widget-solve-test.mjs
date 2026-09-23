// Simulate the exact widget flow: fetch challenge, solve via the fixed driver
import { chromium } from 'playwright';
const exe = '/Users/sabelakhoua/Library/Caches/ms-playwright/chromium-1217/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing';
const fs = await import('fs');
const driver = fs.readFileSync('/Users/sabelakhoua/IdeaProjects/ApexMail/packages/kiwicaptcha-wasm/assets/widget-driver.js', 'utf8');
const wasmEmbed = fs.readFileSync('/Users/sabelakhoua/IdeaProjects/ApexMail/packages/kiwicaptcha-wasm/assets/kiwicaptcha-wasm.js', 'utf8');
const css = fs.readFileSync('/Users/sabelakhoua/IdeaProjects/ApexMail/packages/kiwicaptcha-wasm/assets/widget.css', 'utf8');
const browser = await chromium.launch({ executablePath: exe });
const p = await (await browser.newContext()).newPage();
await p.goto('https://app.apexmail.ee/login/', { waitUntil: 'load' });
const r = await p.evaluate(async ({ driver, wasmEmbed, css }) => {
  // Build the widget DOM exactly as the Rust template does
  const wrap = document.createElement('div');
  wrap.innerHTML = `<style>${css}</style>
  <div class="kiwi-container" id="kiwicaptcha-root">
    <div class="kiwi-widget" data-kiwi-widget data-state="idle" role="status" aria-live="polite">
      <div class="kiwi-icon-wrapper"><svg></svg><div class="kiwi-glow"></div></div>
      <div class="kiwi-main">
        <div class="kiwi-top"><span class="kiwi-label" data-kiwi-label>Security Check</span><span class="kiwi-badge" data-kiwi-badge>Idle</span></div>
        <div class="kiwi-track"><div class="kiwi-bar" data-kiwi-bar></div></div>
        <div class="kiwi-bottom"><p class="kiwi-info" data-kiwi-info>Protected</p><span class="kiwi-timer" data-kiwi-timer></span></div>
      </div>
      <input type="hidden" name="kiwi__token" data-kiwi-token value="" />
    </div>
  </div>`;
  document.body.appendChild(wrap);
  // Inject the wasm embed + driver
  const s1 = document.createElement('script'); s1.textContent = wasmEmbed; document.body.appendChild(s1);
  const s2 = document.createElement('script'); s2.textContent = driver; document.body.appendChild(s2);
  const deadline = Date.now() + 30000;
  while (Date.now() < deadline) {
    const w = document.querySelector('[data-kiwi-widget]');
    const token = document.querySelector('[data-kiwi-token]')?.value;
    if (token) {
      const parts = atob(token).split('.');
      return { state: w.getAttribute('data-state'), counter: parts[1], durationMs: parts[2] };
    }
    if (w.getAttribute('data-state') === 'failed') return { state: 'failed' };
    await new Promise(r => setTimeout(r, 200));
  }
  return { state: document.querySelector('[data-kiwi-widget]')?.getAttribute('data-state'), timeout: true };
}, { driver, wasmEmbed, css });
console.log(JSON.stringify(r, null, 1));
await browser.close();
