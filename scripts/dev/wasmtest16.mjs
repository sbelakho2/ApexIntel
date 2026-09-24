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
  const alloc = (bytes) => { const ptr = w.__wbindgen_malloc(bytes.length, 1); new Uint8Array(w.memory.buffer).set(bytes, ptr); return ptr; };
  const free = (ptr, len) => { if (w.__wbindgen_free) w.__wbindgen_free(ptr, len, 1); };
  const out = {};
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
  // memory sanity: repeated alloc/free must not leak
  const before = w.memory.buffer.byteLength;
  for (let i = 0; i < 2000; i++) { const pp = alloc(new Uint8Array(50)); free(pp, 50); }
  out.memStable = w.memory.buffer.byteLength === before;
  return out;
}, asset);
console.log(JSON.stringify(result, null, 1));
await browser.close();
