import { chromium } from 'playwright';
const exe = '/Users/sabelakhoua/Library/Caches/ms-playwright/chromium-1217/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing';
const fs = await import('fs');
const asset = fs.readFileSync('/tmp/release-embed.js', 'utf8');
const browser = await chromium.launch({ executablePath: exe });
const p = await (await browser.newContext()).newPage();
const result = await p.evaluate(async (src) => {
  eval(src);
  const w = await window.__kiwiCaptchaWasm.load();
  const enc = new TextEncoder();
  const alloc = (bytes) => {
    const ptr = w.__wbindgen_malloc(bytes.length, 1);
    new Uint8Array(w.memory.buffer).set(bytes, ptr);
    return ptr;
  };
  const free = (ptr, len) => { if (w.__wbindgen_free) w.__wbindgen_free(ptr, len, 1); };
  const out = {};
  // SHA-256 solve, 8 bits
  const t0 = performance.now();
  let found = -1;
  for (let start = 0; start < 200000; start += 512) {
    const pp = alloc(enc.encode('abc')), sp = alloc(enc.encode('def'));
    found = w.solve_sha256_chunk(pp, 3, sp, 3, 8, start, 512);
    free(pp, 3); free(sp, 3);
    if (found !== -1) break;
  }
  out.shaFound = found;
  out.shaTimeMs = Math.round(performance.now() - t0);
  // verify sha server-style: leading zeros of SHA256('abc'+found+'def')
  const hex = Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256', enc.encode('abc' + found + 'def')))).map(b => b.toString(16).padStart(2,'0')).join('');
  let z = 0;
  for (const ch of hex) { if (ch === '0') { z += 4; continue; } z += ('0000' + parseInt(ch,16).toString(2)).slice(-4).indexOf('1'); break; }
  out.shaZeros = z;
  // Argon2 solve, 4 bits, m_kib=128, valid 8-byte salt
  const t1 = performance.now();
  let arg = -1;
  for (let start = 0; start < 4000; start += 16) {
    const pp = alloc(enc.encode('abc')), sp = alloc(enc.encode('12345678'));
    arg = w.solve_argon2_chunk(pp, 3, sp, 8, 4, 128, 1, 1, start, 16);
    free(pp, 3); free(sp, 8);
    if (arg !== -1) break;
  }
  out.argon2Found = arg;
  out.argon2TimeMs = Math.round(performance.now() - t1);
  return out;
}, asset);
console.log(JSON.stringify(result, null, 1));
await browser.close();
