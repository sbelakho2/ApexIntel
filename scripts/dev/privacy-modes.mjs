import { chromium } from 'playwright';
import http from 'http';
import fs from 'fs';
const exe = '/Users/sabelakhoua/Library/Caches/ms-playwright/chromium-1217/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing';
const driver = fs.readFileSync('/Users/sabelakhoua/IdeaProjects/ApexMail/packages/kiwicaptcha-wasm/assets/widget-driver.js', 'utf8');
const wasm = fs.readFileSync('/Users/sabelakhoua/IdeaProjects/ApexMail/packages/kiwicaptcha-wasm/assets/kiwicaptcha-wasm.js', 'utf8');
const css = fs.readFileSync('/Users/sabelakhoua/IdeaProjects/ApexMail/packages/kiwicaptcha-wasm/assets/widget.css', 'utf8');
const PORT = 8124;
const server = http.createServer(async (req, res) => {
  if (req.method === 'POST' && req.url === '/api/kcaptcha/challenge') {
    let body = '';
    for await (const c of req) body += c;
    try {
      const r = await fetch('https://app.apexmail.ee/api/kcaptcha/challenge', { method: 'POST', headers: { 'content-type': 'application/json' }, body });
      res.writeHead(r.status, { 'content-type': 'application/json' }); res.end(await r.text());
    } catch { res.writeHead(502); res.end('{}'); }
    return;
  }
  if (req.url === '/cross') { // cross-origin endpoint target: the driver must REFUSE it
    res.writeHead(200, { 'content-type': 'application/json' }); res.end('{"error":"should never be fetched"}'); return;
  }
  const mode = new URL(req.url, 'http://x').searchParams.get('mode') || 'off';
  const cross = new URL(req.url, 'http://x').searchParams.get('cross') === '1';
  const endpoint = cross ? 'https://evil.example.com/challenge' : '/api/kcaptcha/challenge';
  res.writeHead(200, { 'content-type': 'text/html' });
  res.end(`<!DOCTYPE html><html><head><style>${css}</style></head><body>
  <div class="kiwi-container" id="kiwicaptcha-root" data-kiwi-endpoint="${endpoint}" data-kiwi-scope="login" data-kiwi-telemetry="${mode}">
    <input type="hidden" name="kiwi__token" data-kiwi-token value="" />
    <div class="kiwi-widget" data-kiwi-widget data-state="idle" role="status" aria-live="polite">
      <div class="kiwi-icon-wrapper"><svg></svg><div class="kiwi-glow"></div></div>
      <div class="kiwi-main"><div class="kiwi-top"><span class="kiwi-label" data-kiwi-label>Security Check</span><span class="kiwi-badge" data-kiwi-badge>Idle</span></div>
        <div class="kiwi-track"><div class="kiwi-bar" data-kiwi-bar></div></div>
        <div class="kiwi-bottom"><p class="kiwi-info" data-kiwi-info>Protected</p><span class="kiwi-timer" data-kiwi-timer></span></div></div>
    </div>
  </div>
  <script>${wasm}</script><script>${driver}</script></body></html>`);
});
await new Promise(r => server.listen(PORT, '127.0.0.1', r));
const browser = await chromium.launch({ executablePath: exe });
const results = {};

for (const mode of ['off', 'minimal', 'full']) {
  const p = await (await browser.newContext()).newPage();
  const external = [];
  p.on('request', r => { const u = new URL(r.url()); if (u.origin !== `http://127.0.0.1:${PORT}`) external.push(r.url()); });
  await p.goto(`http://127.0.0.1:${PORT}/?mode=${mode}`);
  const r = await p.evaluate(async () => {
    const deadline = Date.now() + 30000;
    while (Date.now() < deadline) {
      const t = document.querySelector('[data-kiwi-token]');
      const w = document.querySelector('[data-kiwi-widget]');
      if (t && t.value) {
        const parts = atob(t.value).split('.');
        let telemetry = {};
        try { telemetry = JSON.parse(parts[3] || '{}'); } catch { telemetry = { parse: 'fail' }; }
        return { state: w?.getAttribute('data-state'), telemetry };
      }
      if (w?.getAttribute('data-state') === 'failed') return { state: 'failed' };
      await new Promise(r2 => setTimeout(r2, 200));
    }
    return { timeout: true };
  });
  results[mode] = { ...r, externalRequests: external };
  await p.close();
}

// Same-origin refusal: the container is SERVED with a cross-origin
// endpoint; the driver must refuse it at init (widget fails, no request
// ever leaves for the foreign origin).
const p2 = await (await browser.newContext()).newPage();
const external2 = [];
p2.on('request', r => { const u = new URL(r.url()); if (u.origin !== `http://127.0.0.1:${PORT}`) external2.push(r.url()); });
await p2.goto(`http://127.0.0.1:${PORT}/?cross=1`);
const refusal = await p2.evaluate(async () => {
  const deadline = Date.now() + 8000;
  const w = document.querySelector('[data-kiwi-widget]');
  while (Date.now() < deadline) {
    if (w.getAttribute('data-state') === 'failed') return { refused: true, state: 'failed' };
    if (w.getAttribute('data-state') === 'done') return { refused: false, state: 'done' };
    await new Promise(r => setTimeout(r, 200));
  }
  return { refused: false, state: w.getAttribute('data-state') };
});
results['same-origin-refusal'] = { ...refusal, externalRequests: external2 };
await p2.close();

console.log(JSON.stringify(results, null, 1));
await browser.close();
server.close();
