import { chromium } from 'playwright';
const exe = '/Users/sabelakhoua/Library/Caches/ms-playwright/chromium-1217/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing';
const fs = await import('fs');
const asset = fs.readFileSync('/Users/sabelakhoua/IdeaProjects/ApexMail/packages/kiwicaptcha-wasm/assets/kiwicaptcha-wasm.js', 'utf8');
const browser = await chromium.launch({ executablePath: exe });
const p = await (await browser.newContext()).newPage();
const result = await p.evaluate(async (src) => {
  eval(src);
  const w = await window.__kiwiCaptchaWasm.load();
  // Test SHA-256: prefix "abc", salt "def", target 8 bits — expect a counter within ~300 tries
  const enc = new TextEncoder();
  const t0 = performance.now();
  let found = -1;
  for (let start = 0; start < 100000; start += 512) {
    found = w.solve_sha256_chunk(enc.encode('abc'), enc.encode('def'), 8, start, 512);
    if (found !== -1) break;
  }
  const shaTime = Math.round(performance.now() - t0);
  // Verify the counter server-side style: SHA-256(prefix||counter||salt) leading zeros >= 8
  async function sha256hex(input) {
    const buf = await crypto.subtle.digest('SHA-256', input);
    return Array.from(new Uint8Array(buf)).map(b => b.toString(16).padStart(2, '0')).join('');
  }
  const verifyOk = found !== -1 ? await (async () => {
    const input = enc.encode('abc' + found + 'def');
    const hex = await sha256hex(input);
    let zeros = 0;
    for (const ch of hex) { if (ch === '0') zeros += 4; else { zeros += '0123456789abcdef'.indexOf(ch).toString(2).padStart(4, '0').indexOf('1'); break; } }
    return zeros;
  })() : -1;
  // Test Argon2: m_kib=64 (min 8*p), t=1, p=1, target 4 bits — must find within ~16 tries
  const t1 = performance.now();
  let arg = -1;
  for (let start = 0; start < 2000; start += 32) {
    arg = w.solve_argon2_chunk(enc.encode('abc'), enc.encode('def'), 4, 64, 1, 1, start, 32);
    if (arg !== -1) break;
  }
  const argTime = Math.round(performance.now() - t1);
  return { shaFound: found, shaZeros: verifyOk, shaTimeMs: shaTime, argon2Found: arg, argon2TimeMs: argTime };
}, asset);
console.log(JSON.stringify(result, null, 1));
await browser.close();
