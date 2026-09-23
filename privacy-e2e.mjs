import { chromium } from 'playwright';
import http from 'http';
import fs from 'fs';
const exe = '/Users/sabelakhoua/IdeaProjects/ApexIntel/../../Library/Caches/ms-playwright/chromium-1217/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing';
const driver = fs.readFileSync('/Users/sabelakhoua/IdeaProjects/ApexMail/packages/kiwicaptcha-wasm/assets/widget-driver.js', 'utf8');
const wasm = fs.readFileSync('/Users/sabelakhoua/IdeaProjects/ApexMail/packages/kiwicaptcha-wasm/assets/kiwicaptcha-wasm.js', 'utf8');
const css = fs.readFileSync('/Users/sabelakhoua/IdeaProjects/ApexMail/packages/kiwicaptcha-wasm/assets/widget.css', 'utf8');
const PORT = 8123;
const server = http.createServer(async (req, res) => {
  if (req.method === 'POST' && req.url === '/api/kcaptcha/challenge') {
    let body = '';
    for await (const c of req) body += c;
    try {
      const r = await fetch('https://app.apexmail.ee/api/kcaptcha/challenge', { method: 'POST', headers: { 'content-type': 'application/json' }, body });
      const data = await r.text();
      res.writeHead(r.status, { 'content-type': 'application/json' });
      res.end(data);
    } catch (e) { res.writeHead(502); res.end('{}'); }
    return;
  }
  res.writeHead(200, { 'content-type': 'text/html' });
  res.end(`<!DOCTYPE html><html><head><style>${css}</style></head><body>
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
  </div>
  <script>${wasm}</script><script>${driver}</script></body></html>`);
});
await new Promise(r => server.listen(PORT, '127.0.0.1', r));
const browser = await chromium.launch({ executablePath: exe });
const p = await (await browser.newContext()).newPage();
const external = [];
p.on('request', r => { const u = new URL(r.url()); if (u.origin !== `http://127.0.0.1:${PORT}` && u.origin !== 'http://localhost:8123') external.push(r.url()); });
await p.goto(`http://127.0.0.1:${PORT}/`);
const r = await p.evaluate(async () => {
  const deadline = Date.now() + 30000;
  while (Date.now() < deadline) {
    const t = document.querySelector('[data-kiwi-token]');
    const w = document.querySelector('[data-kiwi-widget]');
    if (t && t.value) {
      const parts = atob(t.value).split('.');
      let telemetry = {};
      try { telemetry = JSON.parse(parts[3] || '{}'); } catch { telemetry = { parse: 'fail' }; }
      const banned = ['hc', 'dm', 'sw', 'sh', 'et', 'wd'].filter(k => k in telemetry);
      return { state: w?.getAttribute('data-state'), counter: parts[1], telemetryKeys: Object.keys(telemetry), telemetry, bannedPresent: banned, tokenLen: t.value.length };
    }
    await new Promise(r2 => setTimeout(r2, 200));
  }
  return { timeout: true, state: document.querySelector('[data-kiwi-widget]')?.getAttribute('data-state') };
});
console.log(JSON.stringify({ result: r, externalRequests: external }, null, 1));
await browser.close();
server.close();
