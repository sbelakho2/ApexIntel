import { chromium } from 'playwright';
const exe = '/Users/sabelakhoua/Library/Caches/ms-playwright/chromium-1217/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing';
const fs = await import('fs');
const asset = fs.readFileSync('/tmp/debug-embed.js', 'utf8');
const browser = await chromium.launch({ executablePath: exe });
const p = await (await browser.newContext()).newPage();
const result = await p.evaluate(async (src) => {
  eval(src);
  const w = await window.__kiwiCaptchaWasm.load();
  const out = {};
  try { out.slice = w.probe_slice(new Uint8Array([1,2,3])); } catch (e) { out.sliceErr = String(e); }
  try { out.vec = w.probe_vec_owned(new Uint8Array([1,2,3])); } catch (e) { out.vecErr = String(e); }
  try { out.sha = w.solve_sha256_chunk(new Uint8Array([97,98,99]), new Uint8Array([100,101,102]), 4, 0, 64); } catch (e) { out.shaErr = String(e); }
  return out;
}, asset);
console.log(JSON.stringify(result, null, 1));
await browser.close();
