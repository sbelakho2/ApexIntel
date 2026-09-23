import { chromium } from 'playwright';
const exe = '/Users/sabelakhoua/Library/Caches/ms-playwright/chromium-1217/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing';
const fs = await import('fs');
const asset = fs.readFileSync('/Users/sabelakhoua/IdeaProjects/ApexMail/packages/kiwicaptcha-wasm/assets/kiwicaptcha-wasm.js', 'utf8');
const browser = await chromium.launch({ executablePath: exe });
const p = await (await browser.newContext()).newPage();
const result = await p.evaluate(async (src) => {
  // Load the wasm solver
  eval(src);
  const w = await window.__kiwiCaptchaWasm.load();
  const enc = new TextEncoder();
  const alloc = (b) => { const ptr = w.__wbindgen_malloc(b.length, 1); new Uint8Array(w.memory.buffer).set(b, ptr); return ptr; };
  const free = (ptr, len) => w.__wbindgen_free(ptr, len, 1);

  // 1. Get CSRF from login page
  const pageResp = await fetch('https://app.apexmail.ee/login/', { credentials: 'include' });
  const html = await pageResp.text();
  const m = html.match(/name="csrf_token" value="([^"]+)"/);
  const csrf = m ? m[1] : null;

  // 2. Challenge
  const cResp = await fetch('https://app.apexmail.ee/api/kcaptcha/challenge', {
    method: 'POST', headers: {'Content-Type':'application/json'}, body: JSON.stringify({scope:'login'}), credentials: 'include'
  });
  const d = await cResp.json();

  // 3. Solve with wasm (20-bit)
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

  // 4. Token + real login with wrong password
  const plain = d.nonce + '.' + counter + '.' + duration + '.' + JSON.stringify({wd:false, me:3, ke:1, et:[100,250,480]});
  const token = btoa(plain);
  const loginResp = await fetch('https://app.apexmail.ee/v1/auth/login', {
    method: 'POST',
    headers: {'Content-Type':'application/json', 'X-CSRF-Token': csrf, 'Origin':'https://app.apexmail.ee'},
    body: JSON.stringify({email:'sabelakho@apexmail.ee', password:'definitely-wrong-password', kiwi__token: token}),
    credentials: 'include'
  });
  const body = await loginResp.text();
  return { csrf: !!csrf, algorithm: d.algorithm, counter, durationMs: duration, loginStatus: loginResp.status, loginBody: body.slice(0, 150) };
}, asset);
console.log(JSON.stringify(result, null, 1));
await browser.close();
