// T3/T5/T6 rerun with 15s spacing (well under login zone 10r/m).
import { chromium } from 'playwright';
const exe = '/Users/sabelakhoua/Library/Caches/ms-playwright/chromium-1217/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing';
const fs = await import('fs');
const asset = fs.readFileSync('/Users/sabelakhoua/IdeaProjects/ApexMail/packages/kiwicaptcha-wasm/assets/kiwicaptcha-wasm.js', 'utf8');
const browser = await chromium.launch({ executablePath: exe, proxy: { server: "socks5://127.0.0.1:1080" } });
const results = [];
const log = (name, pass, detail) => { results.push({ name, pass }); console.log(`${pass ? 'PASS' : 'FAIL'}  ${name}  ${detail || ''}`); };
const sleep = ms => new Promise(r => setTimeout(r, ms));
const ctx = await browser.newContext();
await ctx.addInitScript(() => {
  Object.defineProperty(navigator, 'webdriver', { get: () => false });
  Object.defineProperty(navigator, 'plugins', { get: () => [1,2,3,4,5] });
  Object.defineProperty(navigator, 'languages', { get: () => ['en-US','en'] });
  window.chrome = { runtime: {} };
});
const p = await ctx.newPage();
await p.goto('https://app.apexmail.ee/login/', { waitUntil: 'load' });
const getChallenge = () => p.evaluate(async () => {
  const r = await fetch('/api/kcaptcha/challenge', { method:'POST', headers:{'Content-Type':'application/json'}, body: JSON.stringify({scope:'login'}) });
  return { ok: r.ok, status: r.status, data: r.ok ? await r.json() : null };
});
const solveWasm = (d) => p.evaluate(async ({ src, d }) => {
  eval(src);
  const w = await window.__kiwiCaptchaWasm.load();
  const enc = new TextEncoder();
  const alloc = (b) => { const ptr = w.__wbindgen_malloc(b.length, 1); new Uint8Array(w.memory.buffer).set(b, ptr); return ptr; };
  const free = (ptr, len) => w.__wbindgen_free(ptr, len, 1);
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
  return { counter, durationMs: Math.round(performance.now() - t0) };
}, { src: asset, d });
const submitLogin = (token) => p.evaluate(async ({ token }) => {
  const csrf = document.querySelector('input[name="_csrf"]')?.value;
  const r = await fetch('/v1/auth/login', {
    method: 'POST',
    headers: {'Content-Type':'application/json', 'X-CSRF-Token': csrf},
    body: JSON.stringify({email:'sabelakho@apexmail.ee', password:'definitely-wrong-password', kiwi__token: token})
  });
  return { status: r.status, body: await r.text() };
}, { token });
const makeToken = (nonce, counter, durationMs, telemetry) =>
  btoa(`${nonce}.${counter}.${durationMs}.${JSON.stringify(telemetry)}`);
const realTelemetry = { wd:false, me:5, ke:2, et:[100,187,296,412,531] };

// T3 uniform timing
{
  const c = await getChallenge();
  const sol = await solveWasm(c.data);
  const uniform = Array.from({length: 30}, (_, i) => 100 + i * 100);
  const r = await submitLogin(makeToken(c.data.nonce, sol.counter, sol.durationMs, { wd:false, me:20, ke:0, et:uniform }));
  log('T3 uniform-timing bot rejected', r.status === 400 && r.body.includes('CAPTCHA'), `status=${r.status}`);
}
await sleep(15000);
// T5 tampered counter
{
  const c = await getChallenge();
  const sol = await solveWasm(c.data);
  const r = await submitLogin(makeToken(c.data.nonce, sol.counter + 1, sol.durationMs, realTelemetry));
  log('T5 tampered counter rejected', r.status === 400 && r.body.includes('CAPTCHA'), `status=${r.status}`);
}
await sleep(15000);
// T6 forged nonce
{
  const c = await getChallenge();
  const sol = await solveWasm(c.data);
  const r = await submitLogin(makeToken('AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=', sol.counter, sol.durationMs, realTelemetry));
  log('T6 forged nonce rejected', r.status === 400 && r.body.includes('CAPTCHA'), `status=${r.status}`);
}
await ctx.close();
console.log('\n=== SUMMARY ===');
const passed = results.filter(r => r.pass).length;
console.log(`${passed}/${results.length} passed`);
await browser.close();
process.exit(results.every(r => r.pass) ? 0 : 1);
