import { chromium } from 'playwright';
const exe = '/Users/sabelakhoua/Library/Caches/ms-playwright/chromium-1217/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing';
const fs = await import('fs');
const asset = fs.readFileSync('/tmp/debug-embed.js', 'utf8');
const browser = await chromium.launch({ executablePath: exe });
const p = await (await browser.newContext()).newPage();
p.on('console', m => { if (m.type() === 'error') console.log('CONSOLE-ERR:', m.text().slice(0, 200)); });
const result = await p.evaluate(async (src) => {
  eval(src);
  const w = await window.__kiwiCaptchaWasm.load();
  w.init_panic_hook();
  const enc = new TextEncoder();
  const out = {};
  try { out.params = w.probe_params(64, 1, 1); } catch (e) { out.paramsErr = String(e); }
  try { out.vec = w.probe_vec(enc.encode('abc')); } catch (e) { out.vecErr = String(e); }
  try { out.hash = w.probe_hash(enc.encode('abc'), enc.encode('12345678')); } catch (e) { out.hashErr = String(e); }
  return out;
}, asset);
console.log(JSON.stringify(result, null, 1));
await browser.close();
