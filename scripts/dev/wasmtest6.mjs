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
  // Valid salt (8 bytes)
  try { out.argValidSalt = w.solve_argon2_chunk(enc.encode('abc'), enc.encode('12345678'), 4, 64, 1, 1, 0, 16); }
  catch (e) { out.argValidSaltErr = String(e); }
  // Larger m_kib
  try { out.arg1024 = w.solve_argon2_chunk(enc.encode('abc'), enc.encode('12345678'), 4, 1024, 1, 1, 0, 16); }
  catch (e) { out.arg1024Err = String(e); }
  return out;
}, asset);
console.log(JSON.stringify(result, null, 1));
await browser.close();
