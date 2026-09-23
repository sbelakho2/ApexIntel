// Final aggressive matrix — combined logins to stay under 10r/m login zone.
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

// T1+T4: legit pass + replay reject (2 logins, same token)
{
  const c = await getChallenge();
  const sol = await solveWasm(c.data);
  const token = makeToken(c.data.nonce, sol.counter, sol.durationMs, realTelemetry);
  const r1 = await submitLogin(token);
  await sleep(6500);
  const r2 = await submitLogin(token);
  log('T1 legit captcha passes (401=wrong pw)', r1.status === 401, `status=${r1.status} solve=${sol.durationMs}ms`);
  log('T4 replay rejected', r2.status === 400 && (r2.body.includes('already used') || r2.body.includes('CAPTCHA')), `status=${r2.status} body=${r2.body.slice(0,60)}`);
}
await sleep(6500);

// T2: webdriver bot
{
  const c = await getChallenge();
  const sol = await solveWasm(c.data);
  const r = await submitLogin(makeToken(c.data.nonce, sol.counter, sol.durationMs, { wd:true, me:0, ke:0, et:[] }));
  log('T2 webdriver bot rejected', r.status === 400 && r.body.includes('CAPTCHA'), `status=${r.status}`);
}
await sleep(6500);

// T3: uniform timing
{
  const c = await getChallenge();
  const sol = await solveWasm(c.data);
  const uniform = Array.from({length: 30}, (_, i) => 100 + i * 100);
  const r = await submitLogin(makeToken(c.data.nonce, sol.counter, sol.durationMs, { wd:false, me:20, ke:0, et:uniform }));
  log('T3 uniform-timing bot rejected', r.status === 400 && r.body.includes('CAPTCHA'), `status=${r.status}`);
}
await sleep(6500);

// T5: tampered counter
{
  const c = await getChallenge();
  const sol = await solveWasm(c.data);
  const r = await submitLogin(makeToken(c.data.nonce, sol.counter + 1, sol.durationMs, realTelemetry));
  log('T5 tampered counter rejected', r.status === 400 && r.body.includes('CAPTCHA'), `status=${r.status}`);
}
await sleep(6500);

// T6: forged nonce
{
  const c = await getChallenge();
  const sol = await solveWasm(c.data);
  const r = await submitLogin(makeToken('AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=', sol.counter, sol.durationMs, realTelemetry));
  log('T6 forged nonce rejected', r.status === 400 && r.body.includes('CAPTCHA'), `status=${r.status}`);
}
await sleep(6500);

// T7: zero-duration
{
  const c = await getChallenge();
  const sol = await solveWasm(c.data);
  const r = await submitLogin(makeToken(c.data.nonce, sol.counter, 0, realTelemetry));
  log('T7 zero-duration rejected', r.status === 400 && r.body.includes('CAPTCHA'), `status=${r.status}`);
}
await sleep(6500);

// T8: perf 5x
{
  const times = [];
  for (let i = 0; i < 5; i++) {
    const c = await getChallenge();
    const sol = await solveWasm(c.data);
    times.push(sol.durationMs);
    await sleep(6000);
  }
  const avg = Math.round(times.reduce((a,b)=>a+b,0) / times.length);
  log('T8 WASM 20-bit perf', avg < 1000, `avg=${avg}ms [${times.join(',')}]`);
}

// T9: Argon2id solver
{
  const perf = await p.evaluate(async ({ src }) => {
    eval(src);
    const w = await window.__kiwiCaptchaWasm.load();
    const enc = new TextEncoder();
    const alloc = (b) => { const ptr = w.__wbindgen_malloc(b.length, 1); new Uint8Array(w.memory.buffer).set(b, ptr); return ptr; };
    const free = (ptr, len) => w.__wbindgen_free(ptr, len, 1);
    const prefix = enc.encode('bench-argon2-prefix|login|');
    const salt = enc.encode('0123456789abcdef');
    const t0 = performance.now();
    let found = -1;
    for (let start = 0; start < 5000; start += 16) {
      const pp = alloc(prefix), sp = alloc(salt);
      found = w.solve_argon2_chunk(pp, prefix.length, sp, salt.length, 8, 8192, 1, 1, start, 16);
      free(pp, prefix.length); free(sp, salt.length);
      if (found !== -1) break;
    }
    return { found, ms: Math.round(performance.now() - t0) };
  }, { src: asset });
  log('T9 Argon2id 8-bit @ 8MiB', perf.found >= 0 && perf.ms < 5000, `found=${perf.found} ${perf.ms}ms`);
}

// T10: widget auto-solve full pipeline
{
  await p.reload({ waitUntil: 'load' });
  const widget = await p.evaluate(async () => {
    const deadline = Date.now() + 60000;
    while (Date.now() < deadline) {
      const state = document.querySelector('[data-kiwi-widget]')?.getAttribute('data-state');
      const token = document.getElementById('kiwi-token-input')?.value;
      if (token) {
        const parts = atob(token).split('.');
        return { state, durationMs: parts[2], tokenLen: token.length };
      }
      if (state === 'failed') return { state: 'failed' };
      await new Promise(r => setTimeout(r, 200));
    }
    return { timeout: true };
  });
  log('T10 widget full pipeline', widget.tokenLen > 50 && widget.state === 'done', JSON.stringify(widget));
}

await ctx.close();
console.log('\n=== SUMMARY ===');
const passed = results.filter(r => r.pass).length;
console.log(`${passed}/${results.length} passed`);
await browser.close();
process.exit(results.every(r => r.pass) ? 0 : 1);
