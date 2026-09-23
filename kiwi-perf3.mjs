import { chromium } from 'playwright';
import { createHash } from 'node:crypto';
const exe = '/Users/sabelakhoua/Library/Caches/ms-playwright/chromium-1217/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing';
const fs = await import('fs');
const asset = fs.readFileSync('/Users/sabelakhoua/IdeaProjects/ApexMail/packages/kiwicaptcha-wasm/assets/kiwicaptcha-wasm.js', 'utf8');
const browser = await chromium.launch({ executablePath: exe });
const p = await (await browser.newContext()).newPage();
const result = await p.evaluate(async ({ src }) => {
  eval(src);
  const w = await window.__kiwiCaptchaWasm.load();
  const enc = new TextEncoder();
  const alloc = (b) => { const ptr = w.alloc ? w.alloc(b.length) : w.__wbindgen_malloc(b.length, 1); new Uint8Array(w.memory.buffer).set(b, ptr); return ptr; };
  const free = (ptr, len) => { if (w.dealloc) w.dealloc(ptr, len); else w.__wbindgen_free(ptr, len, 1); };
  const out = {};
  const prefix = enc.encode('perftest|login|'), salt = enc.encode('0123456789abcdef');
  const t0 = performance.now();
  let found = -1;
  for (let start = 0; start < 100000; start += 512) {
    const pp = alloc(prefix), sp = alloc(salt);
    found = w.solve_sha256_chunk(pp, prefix.length, sp, salt.length, 8, start, 512);
    free(pp, prefix.length); free(sp, salt.length);
    if (found !== -1) break;
  }
  out.sha8bitFound = found;
  out.sha8bitMs = Math.round(performance.now() - t0);
  const shaTimes = [];
  for (let i = 0; i < 5; i++) {
    const pref = enc.encode(`perf|login|${i}|`), slt = enc.encode('0123456789abcdef');
    const t = performance.now();
    let f = -1;
    for (let start = 0; start < 3000000; start += 50000) {
      const pp = alloc(pref), sp = alloc(slt);
      f = w.solve_sha256_chunk(pp, pref.length, sp, slt.length, 20, start, 50000);
      free(pp, pref.length); free(sp, slt.length);
      if (f !== -1) break;
    }
    shaTimes.push(Math.round(performance.now() - t));
  }
  out.sha20bit = shaTimes;
  out.sha20bitAvg = Math.round(shaTimes.reduce((a,b)=>a+b,0)/shaTimes.length);
  const argTimes = [];
  for (let i = 0; i < 3; i++) {
    const pref = enc.encode(`perf|login|${i}|`), slt = enc.encode('0123456789abcdef');
    const t = performance.now();
    let f = -1;
    for (let start = 0; start < 5000; start += 16) {
      const pp = alloc(pref), sp = alloc(slt);
      f = w.solve_argon2_chunk(pp, pref.length, sp, slt.length, 8, 8192, 1, 1, start, 16);
      free(pp, pref.length); free(sp, slt.length);
      if (f !== -1) break;
    }
    argTimes.push(Math.round(performance.now() - t));
  }
  out.argon2_8bit_8MiB = argTimes;
  out.argon2Avg = Math.round(argTimes.reduce((a,b)=>a+b,0)/argTimes.length);
  const before = w.memory.buffer.byteLength;
  for (let i = 0; i < 1000; i++) { const pp = alloc(new Uint8Array(64)); free(pp, 64); }
  out.memStable = w.memory.buffer.byteLength === before;
  return out;
}, { src: asset });
// Independent verification in node: leading zeros of sha256(prefix||counter||salt)
const prefix = 'perftest|login|';
const salt = '0123456789abcdef';
const counter = result.sha8bitFound;
const h = createHash('sha256').update(prefix + counter + salt).digest();
let zeros = 0;
for (const byte of h) {
  if (byte === 0) { zeros += 8; continue; }
  let b = byte; while ((b & 128) === 0) { zeros++; b <<= 1; } break;
}
result.shaVerifiedZeros = zeros;
console.log(JSON.stringify(result, null, 1));
console.log(`\nIndependent verify: counter=${counter} has ${zeros} leading zero bits (target 8) => ${zeros >= 8 ? 'CORRECT' : 'WRONG'}`);
await browser.close();
