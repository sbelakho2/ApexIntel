// Performance benchmark: optimized wasm (wasm-opt -O + persistent buffers).
import { chromium } from 'playwright';
const exe = '/Users/sabelakhoua/Library/Caches/ms-playwright/chromium-1217/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing';
const fs = await import('fs');
const asset = fs.readFileSync('/Users/sabelakhoua/IdeaProjects/ApexMail/packages/kiwicaptcha-wasm/assets/kiwicaptcha-wasm.js', 'utf8');
const browser = await chromium.launch({ executablePath: exe });
const p = await (await browser.newContext()).newPage();
const result = await p.evaluate(async ({ src }) => {
  eval(src);
  const w = await window.__kiwiCaptchaWasm.load();
  const enc = new TextEncoder();
  const alloc = (b) => { const ptr = w.__wbindgen_malloc(b.length, 1); new Uint8Array(w.memory.buffer).set(b, ptr); return ptr; };
  const free = (ptr, len) => w.__wbindgen_free(ptr, len, 1);
  const out = {};
  // SHA-256 20-bit, 10 runs
  const shaTimes = [];
  for (let i = 0; i < 10; i++) {
    const prefix = enc.encode(`bench|login|${i}|`);
    const salt = enc.encode('0123456789abcdef');
    const t0 = performance.now();
    let found = -1;
    for (let start = 0; start < 3000000; start += 50000) {
      const pp = alloc(prefix), sp = alloc(salt);
      found = w.solve_sha256_chunk(pp, prefix.length, sp, salt.length, 20, start, 50000);
      free(pp, prefix.length); free(sp, salt.length);
      if (found !== -1) break;
    }
    shaTimes.push(Math.round(performance.now() - t0));
  }
  out.sha20bit = shaTimes;
  out.sha20bitAvg = Math.round(shaTimes.reduce((a,b)=>a+b,0)/shaTimes.length);
  // Argon2id 8-bit @ 8 MiB, 3 runs
  const argTimes = [];
  for (let i = 0; i < 3; i++) {
    const prefix = enc.encode(`bench|login|${i}|`), salt = enc.encode('0123456789abcdef');
    const t1 = performance.now();
    let found = -1;
    for (let start = 0; start < 5000; start += 16) {
      const pp = alloc(prefix), sp = alloc(salt);
      found = w.solve_argon2_chunk(pp, prefix.length, sp, salt.length, 8, 8192, 1, 1, start, 16);
      free(pp, prefix.length); free(sp, salt.length);
      if (found !== -1) break;
    }
    argTimes.push(Math.round(performance.now() - t1));
  }
  out.argon2_8bit_8MiB = argTimes;
  out.argon2Avg = Math.round(argTimes.reduce((a,b)=>a+b,0)/argTimes.length);
  // Memory stability: 5000 alloc/free cycles must not grow the heap
  const before = w.memory.buffer.byteLength;
  for (let i = 0; i < 5000; i++) { const pp = alloc(new Uint8Array(64)); free(pp, 64); }
  out.memStable = w.memory.buffer.byteLength === before;
  return out;
}, { src: asset });
console.log(JSON.stringify(result, null, 1));
await browser.close();
