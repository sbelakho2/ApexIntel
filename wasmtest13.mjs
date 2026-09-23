import { chromium } from 'playwright';
const exe = '/Users/sabelakhoua/Library/Caches/ms-playwright/chromium-1217/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing';
const fs = await import('fs');
const asset = fs.readFileSync('/tmp/debug-embed.js', 'utf8');
const browser = await chromium.launch({ executablePath: exe });
const p = await (await browser.newContext()).newPage();
let logs = [];
p.on('console', m => logs.push(m.type() + ': ' + m.text()));
const result = await p.evaluate(async (src) => {
  eval(src);
  const w = await window.__kiwiCaptchaWasm.load();
  w.init_panic_hook();
  try { return { vec: w.probe_vec(new Uint8Array([65,66,67])) }; }
  catch (e) { return { err: String(e) }; }
}, asset);
console.log(JSON.stringify(result, null, 1));
console.log('=== console logs ===');
logs.forEach(l => console.log(l.slice(0, 500)));
await browser.close();
