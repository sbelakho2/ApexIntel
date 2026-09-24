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
  const shaInput = (c) => enc.encode('abc' + c + 'def');
  async function sha256hex(input) {
    const buf = await crypto.subtle.digest('SHA-256', input);
    return Array.from(new Uint8Array(buf)).map(b => b.toString(16).padStart(2, '0')).join('');
  }
  async function leadingZeros(input) {
    const hex = await sha256hex(input);
    let zeros = 0;
    for (const ch of hex) {
      if (ch === '0') { zeros += 4; continue; }
      zeros += ('0000' + parseInt(ch, 16).toString(2)).slice(-4).indexOf('1');
      break;
    }
    return zeros;
  }
  const out = {};
  // SHA-256 solve, 8 bits
  const t0 = performance.now();
  let found = -1;
  for (let start = 0; start < 200000; start += 512) {
    found = w.solve_sha256_chunk(enc.encode('abc'), enc.encode('def'), 8, start, 512);
    if (found !== -1) break;
  }
  out.shaFound = found;
  out.shaZeros = found !== -1 ? await leadingZeros(shaInput(found)) : null;
  out.shaTimeMs = Math.round(performance.now() - t0);
  // Argon2 solve, 4 bits, m_kib=64
  const t1 = performance.now();
  let arg = -1;
  for (let start = 0; start < 4000; start += 16) {
    arg = w.solve_argon2_chunk(enc.encode('abc'), enc.encode('def'), 4, 64, 1, 1, start, 16);
    if (arg !== -1) break;
  }
  out.argon2Found = arg;
  out.argon2TimeMs = Math.round(performance.now() - t1);
  return out;
}, asset);
console.log(JSON.stringify(result, null, 1));
await browser.close();
