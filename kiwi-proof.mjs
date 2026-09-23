import { chromium } from 'playwright';
const exe = '/Users/sabelakhoua/Library/Caches/ms-playwright/chromium-1217/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing';
const fs = await import('fs');
const asset = fs.readFileSync('/Users/sabelakhoua/IdeaProjects/ApexMail/packages/kiwicaptcha-wasm/assets/kiwicaptcha-wasm.js', 'utf8');
const browser = await chromium.launch({ executablePath: exe });
// Use a real browser context so cookies/CORS work naturally against the real origin
const ctx = await browser.newContext();
const p = await ctx.newPage();
await p.goto('https://app.apexmail.ee/login/', { waitUntil: 'load' });
const result = await p.evaluate(async (src) => {
  eval(src);
  const w = await window.__kiwiCaptchaWasm.load();
  const enc = new TextEncoder();
  const alloc = (b) => { const ptr = w.__wbindgen_malloc(b.length, 1); new Uint8Array(w.memory.buffer).set(b, ptr); return ptr; };
  const free = (ptr, len) => w.__wbindgen_free(ptr, len, 1);

  // CSRF from the SSR page
  const csrfMeta = document.querySelector('meta[name="X-CSRF-Token"]');
  const csrfInput = document.querySelector('input[name="_csrf"]');
  const csrf = (csrfMeta && csrfMeta.content) || (csrfInput && csrfInput.value);

  // Challenge (same-origin, cookies set)
  const cResp = await fetch('/api/kcaptcha/challenge', { method: 'POST', headers: {'Content-Type':'application/json'}, body: JSON.stringify({scope:'login'}) });
  const d = await cResp.json();

  // Solve via wasm
  const prefixBytes = enc.encode(d.prefix);
  const saltBytes = Uint8Array.from(atob(d.salt), c => c.charCodeAt(0));
  const t0 = performance.now();
  let counter = -1;
  for (let start = 0; start < 5000000; start += 50000) {
    const pp = alloc(prefixBytes), sp = alloc(saltBytes);
    counter = w.solve_sha256_chunk(pp, prefixBytes.length, sp, saltBytes.length, d.targetBits, start, 50000);
    free(pp, prefixBytes.length); free(sp, saltBytes.length);
    if (counter !== -1) break;
  }
  const duration = Math.round(performance.now() - t0);

  // Token + login (wrong password — captcha must pass, credentials fail)
  const plain = d.nonce + '.' + counter + '.' + duration + '.' + JSON.stringify({wd:false, me:3, ke:1, et:[100,250,480]});
  const token = btoa(plain);
  const loginResp = await fetch('/v1/auth/login', {
    method: 'POST',
    headers: {'Content-Type':'application/json', 'X-CSRF-Token': csrf},
    body: JSON.stringify({email:'sabelakho@apexmail.ee', password:'definitely-wrong-password', kiwi__token: token})
  });
  const body = await loginResp.text();
  return { csrfFound: !!csrf, algorithm: d.algorithm, counter, durationMs: duration, loginStatus: loginResp.status, loginBody: body.slice(0, 200) };
}, asset);
console.log(JSON.stringify(result, null, 1));
await browser.close();
