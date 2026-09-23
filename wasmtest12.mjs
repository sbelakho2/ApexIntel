import { chromium } from 'playwright';
const exe = '/Users/sabelakhoua/Library/Caches/ms-playwright/chromium-1217/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing';
const fs = await import('fs');
const asset = fs.readFileSync('/tmp/debug-embed.js', 'utf8');
const browser = await chromium.launch({ executablePath: exe });
const p = await (await browser.newContext()).newPage();
const result = await p.evaluate(async (src) => {
  eval(src);
  const w = await window.__kiwiCaptchaWasm.load();
  const enc = new TextEncoder();
  const out = {};
  // Direct malloc probe
  try {
    const p0 = w.__wbindgen_malloc(3, 1);
    out.mallocPtr = p0;
    const mem = new Uint8Array(w.memory.buffer);
    mem.set(enc.encode('abc'), p0);
    out.memRead = String.fromCharCode(mem[p0], mem[p0+1], mem[p0+2]);
    w.__wbindgen_free(p0, 3, 1);
  } catch (e) { out.mallocErr = String(e); }
  // Empty vec probe
  try { out.vecEmpty = w.probe_vec(new Uint8Array(0)); } catch (e) { out.vecEmptyErr = String(e); }
  // Non-empty vec probe
  try { out.vecAbc = w.probe_vec(enc.encode('abc')); } catch (e) { out.vecAbcErr = String(e); }
  return out;
}, asset);
console.log(JSON.stringify(result, null, 1));
await browser.close();
