import { chromium } from 'playwright';
const exe = '/Users/sabelakhoua/Library/Caches/ms-playwright/chromium-1217/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing';
const fs = await import('fs');
const asset = fs.readFileSync('/Users/sabelakhoua/IdeaProjects/ApexMail/packages/kiwicaptcha-wasm/assets/kiwicaptcha-wasm.js', 'utf8');
const browser = await chromium.launch({ executablePath: exe });
const p = await (await browser.newContext()).newPage();
const result = await p.evaluate(async (src) => {
  eval(src);
  const w = await window.__kiwiCaptchaWasm.load();
  const enc = new TextEncoder();
  const out = {};
  // SHA alone
  try {
    const r = w.solve_sha256_chunk(enc.encode('abc'), enc.encode('def'), 8, 0, 512);
    out.sha = r;
  } catch (e) { out.shaErr = String(e); }
  // Argon2 alone with tiny params
  try {
    const r = w.solve_argon2_chunk(enc.encode('abc'), enc.encode('def'), 4, 64, 1, 1, 0, 16);
    out.arg = r;
  } catch (e) { out.argErr = String(e); }
  // memory info
  out.memPages = w.memory ? Math.round(w.memory.buffer.byteLength / 65536) : 'n/a';
  return out;
}, asset);
console.log(JSON.stringify(result, null, 1));
await browser.close();
