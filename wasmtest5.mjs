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
  // Check memory.grow limit: wasm-bindgen sets max pages
  try {
    const before = w.memory.buffer.byteLength / 65536;
    const grown = w.memory.grow(1);
    out.growOk = grown;
    out.afterPages = w.memory.buffer.byteLength / 65536;
  } catch (e) { out.growErr = String(e); }
  // Try argon2 with m_kib=64 but catch console for panic message
  const origError = console.error;
  let msgs = [];
  console.error = (...a) => msgs.push(a.join(' '));
  try {
    const r = w.solve_argon2_chunk(enc.encode('abc'), enc.encode('def'), 4, 64, 1, 1, 0, 16);
    out.arg = r;
  } catch (e) { out.argErr = String(e); }
  console.error = origError;
  out.consoleErrors = msgs.slice(0, 3);
  return out;
}, asset);
console.log(JSON.stringify(result, null, 1));
await browser.close();
