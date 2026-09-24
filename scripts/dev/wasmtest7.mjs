import { chromium } from 'playwright';
const exe = '/Users/sabelakhoua/Library/Caches/ms-playwright/chromium-1217/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing';
const fs = await import('fs');
const glue = fs.readFileSync('pkg/kiwicaptcha_wasm.js', 'utf8');
const wasm = fs.readFileSync('pkg/kiwicaptcha_wasm_bg.wasm');
const b64 = wasm.toString('base64');
const asset = glue.replace(/export function/g, 'function').replace(/export\s*\{[^}]*\};\s*$/, '').replace(/if \(module_or_path === undefined\) \{\s*module_or_path = new URL\([^;]+;\s*\}/s, '').replace(/if \(typeof module_or_path === 'string' \|\| \(typeof Request === 'function' && module_or_path instanceof Request\) \|\| \(typeof URL === 'function' && module_or_path instanceof URL\)\) \{\s*module_or_path = fetch\(module_or_path\);\s*\}/s, '');
const browser = await chromium.launch({ executablePath: exe });
const p = await (await browser.newContext()).newPage();
p.on('console', m => console.log('CONSOLE:', m.type(), m.text().slice(0, 300)));
p.on('pageerror', e => console.log('PAGEERROR:', String(e).slice(0, 500)));
const result = await p.evaluate(async ({ src, b64 }) => {
  eval(src);
  const bytes = Uint8Array.from(atob(b64), c => c.charCodeAt(0));
  const w = await __wbg_init(bytes);
  const enc = new TextEncoder();
  try { return { arg: w.solve_argon2_chunk(enc.encode('abc'), enc.encode('12345678'), 4, 64, 1, 1, 0, 16) }; }
  catch (e) { return { err: String(e) }; }
}, { src: asset, b64 });
console.log(JSON.stringify(result, null, 1));
await browser.close();
