import { chromium } from 'playwright';
const exe = '/Users/sabelakhoua/Library/Caches/ms-playwright/chromium-1217/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing';
const fs = await import('fs');
const asset = fs.readFileSync('/tmp/debug-embed.js', 'utf8');
const browser = await chromium.launch({ executablePath: exe });
const p = await (await browser.newContext()).newPage();
p.on('console', m => console.log('CONSOLE:', m.type(), m.text().slice(0, 300)));
const result = await p.evaluate(async (src) => {
  eval(src);
  const w = await window.__kiwiCaptchaWasm.load();
  const enc = new TextEncoder();
  const out = {};
  try { out.argZero = w.solve_argon2_chunk(enc.encode('abc'), enc.encode('12345678'), 4, 64, 1, 1, 0, 0); } catch (e) { out.argZeroErr = String(e); }
  try { out.argOne = w.solve_argon2_chunk(enc.encode('abc'), enc.encode('12345678'), 4, 64, 1, 1, 0, 1); } catch (e) { out.argOneErr = String(e); }
  try { out.argMin = w.solve_argon2_chunk(enc.encode('abc'), enc.encode('12345678'), 4, 8, 1, 1, 0, 1); } catch (e) { out.argMinErr = String(e); }
  return out;
}, asset);
console.log(JSON.stringify(result, null, 1));
await browser.close();
