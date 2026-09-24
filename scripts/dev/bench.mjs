import { chromium } from 'playwright';
const exe = '/Users/sabelakhoua/Library/Caches/ms-playwright/chromium-1217/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing';
const fs = await import('fs');
const newAsset = fs.readFileSync('/Users/sabelakhoua/IdeaProjects/ApexMail/.kilo/worktrees/fix-kiwi-rust-build/packages/kiwicaptcha-wasm/assets/kiwicaptcha-wasm.js', 'utf8');
const browser = await chromium.launch({ executablePath: exe });
const p = await (await browser.newContext()).newPage();
const result = await p.evaluate(async (src) => {
  eval(src);
  const w = await window.__kiwiCaptchaWasm.load();
  const enc = new TextEncoder();
  const alloc = (bytes) => { const ptr = w.__wbindgen_malloc(bytes.length, 1); new Uint8Array(w.memory.buffer).set(bytes, ptr); return ptr; };
  const free = (ptr, len) => { w.__wbindgen_free(ptr, len, 1); };
  const out = {};
  // SHA-256 20-bit (realistic difficulty) — full solve
  const t0 = performance.now();
  let found = -1;
  for (let start = 0; start < 3000000; start += 50000) {
    const pp = alloc(enc.encode('benchprefix|loginsomesalt|')), sp = alloc(enc.encode('0123456789abcdef'));
    found = w.solve_sha256_chunk(pp, 27, sp, 16, 20, start, 50000);
    free(pp, 27); free(sp, 16);
    if (found !== -1) break;
  }
  out.sha20bitMs = Math.round(performance.now() - t0);
  out.shaCounter = found;
  // Argon2id 8-bit, 8 MiB — realistic config
  const t1 = performance.now();
  let arg = -1;
  for (let start = 0; start < 5000; start += 16) {
    const pp = alloc(enc.encode('benchprefix|loginsomesalt|')), sp = alloc(enc.encode('0123456789abcdef'));
    arg = w.solve_argon2_chunk(pp, 27, sp, 16, 8, 8192, 1, 1, start, 16);
    free(pp, 27); free(sp, 16);
    if (arg !== -1) break;
  }
  out.argon2_8bit_8MiB_ms = Math.round(performance.now() - t1);
  out.argon2Counter = arg;
  return out;
}, newAsset);
console.log(JSON.stringify(result, null, 1));
await browser.close();
