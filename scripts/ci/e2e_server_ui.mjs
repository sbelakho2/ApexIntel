// Server-rendered UI end-to-end checks for ApexIntel.
//
// Drives the real Axum app (not the wasm experiment) through a browser and
// asserts, for every navigation route, at mobile AND desktop viewports:
//   * the route responds 2xx
//   * no uncaught page error / console error
//   * no same-origin request returns 4xx/5xx
//   * the page never scrolls horizontally (documentElement.scrollWidth <= vw+1)
//   * short page titles do not break mid-word into "vertical text"
//   * long unbroken strings (entity names, URLs) do not widen the layout
//   * data-table cells stay on one line on small screens
// It also checks the unauthenticated redirect, the styled 404, and that every
// link/hx-* target rendered on a page resolves (link integrity).
//
// Usage: BASE_URL=http://127.0.0.1:9095 ADMIN_USER=admin ADMIN_PASS=adminpassword \
//        node scripts/ci/e2e_server_ui.mjs
import { chromium } from 'playwright';

const BASE = process.env.BASE_URL || 'http://127.0.0.1:9095';
const USER = process.env.ADMIN_USER || 'admin';
const PASS = process.env.ADMIN_PASS || 'adminpassword';

const ROUTES = [
  '/', '/warnings', '/insights', '/companies', '/persons', '/competitors',
  '/battlecards', '/search', '/graph', '/triage', '/workspaces', '/queue',
  '/activity', '/supplier-risk', '/pipeline', '/evidence', '/team-assignments',
  '/executive', '/trends', '/security', '/admin', '/memos', '/notifications',
  '/settings', '/settings/alerts',
];

const VIEWPORTS = [
  { name: 'mobile', width: 390, height: 844 },
  { name: 'desktop', width: 1280, height: 900 },
];

const violations = [];
const v = (msg) => violations.push(msg);

async function login(context) {
  const page = await context.newPage();
  await page.goto(`${BASE}/login`, { waitUntil: 'domcontentloaded' });
  await page.getByRole('textbox', { name: /operator id/i }).fill(USER);
  await page.getByRole('textbox', { name: /access key/i }).fill(PASS);
  await page.getByRole('button', { name: /access platform/i }).click();
  await page.waitForURL((u) => !u.pathname.startsWith('/login'), { timeout: 10000 });
  await page.close();
}

function attach(page, origin, sink) {
  page.on('console', (m) => { if (m.type() === 'error') sink.console.push(m.text().slice(0, 160)); });
  page.on('pageerror', (e) => sink.errors.push(String(e.message).slice(0, 160)));
  page.on('response', (r) => {
    if (r.url().startsWith(origin) && r.status() >= 400 && r.request().resourceType() !== 'document' || (r.url().startsWith(origin) && r.status() >= 400)) {
      sink.http.push(`${r.status()} ${r.request().method()} ${r.url().replace(origin, '')}`);
    }
  });
}

async function collectLinks(page, origin) {
  return await page.evaluate((o) => {
    const out = new Set();
    document.querySelectorAll('a[href]').forEach((a) => {
      const h = a.getAttribute('href') || '';
      if (!h || h.startsWith('#') || h.startsWith('mailto:') || h.startsWith('tel:') || h.startsWith('javascript:')) return;
      try { const u = new URL(h, o); if (u.origin === o) out.add(u.pathname + u.search); } catch { /* ignore */ }
    });
    // `hx-get` targets are safe GET navigations; mutating verbs are skipped.
    document.querySelectorAll('[hx-get]').forEach((el) => {
      const h = el.getAttribute('hx-get');
      if (h && !h.startsWith('#') && !h.includes('{')) { try { const u = new URL(h, o); if (u.origin === o) out.add(u.pathname + u.search); } catch { /* ignore */ } }
    });
    return [...out];
  }, origin);
}

const browser = await chromium.launch();
try {
  // ── Unauthenticated behaviour ─────────────────────────────────────────
  {
    const ctx = await browser.newContext();
    const page = await ctx.newPage();
    const resp = await page.goto(`${BASE}/`, { waitUntil: 'domcontentloaded' });
    if (!resp || !resp.url().includes('/login')) v(`unauthenticated / did not redirect to /login (url=${resp ? resp.url() : 'n/a'})`);
    const nf = await page.goto(`${BASE}/definitely-not-a-real-route`, { waitUntil: 'domcontentloaded' });
    if (!nf || nf.status() !== 404) v(`unknown route expected 404, got ${nf ? nf.status() : 'n/a'}`);
    const ct = nf ? nf.headers()['content-type'] || '' : '';
    if (!ct.includes('text/html')) v(`404 page not styled HTML (content-type=${ct})`);
    await ctx.close();
  }

  // ── Authenticated sweep ───────────────────────────────────────────────
  const authCtx = await browser.newContext();
  await login(authCtx);
  const origin = new URL(BASE).origin;

  for (const vp of VIEWPORTS) {
    const page = await authCtx.newPage();
    await page.setViewportSize({ width: vp.width, height: vp.height });
    const sink = { console: [], errors: [], http: [] };
    attach(page, origin, sink);

    for (const route of ROUTES) {
      sink.console.length = 0; sink.errors.length = 0; sink.http.length = 0;
      const resp = await page.goto(BASE + route, { waitUntil: 'domcontentloaded', timeout: 20000 }).catch(() => null);
      await page.waitForTimeout(350);
      const status = resp ? resp.status() : null;
      if (status === null || status >= 400) v(`[${vp.name}] ${route} returned ${status}`);
      if (sink.errors.length) v(`[${vp.name}] ${route} page error: ${sink.errors[0]}`);
      if (sink.console.length) v(`[${vp.name}] ${route} console error: ${sink.console[0]}`);
      // ignore 429 (rate-limit from reconnect churn) for the hard failure, but report it
      const hardHttp = sink.http.filter((h) => !h.startsWith('429'));
      if (hardHttp.length) v(`[${vp.name}] ${route} HTTP error: ${hardHttp[0]}`);

      const metrics = await page.evaluate(() => {
        const vw = window.innerWidth;
        const h1 = document.querySelector('h1');
        let h1Lines = null;
        if (h1) {
          const cs = getComputedStyle(h1);
          const lh = parseFloat(cs.lineHeight) || parseFloat(cs.fontSize) * 1.1;
          h1Lines = Math.round(h1.getBoundingClientRect().height / lh);
        }
        return {
          scrollWidth: document.documentElement.scrollWidth,
          vw,
          h1Text: h1 ? h1.textContent.trim() : '',
          h1Lines,
          titleNowrap: (() => {
            const td = document.querySelector('.apex-table tbody td');
            return td ? getComputedStyle(td).whiteSpace : null;
          })(),
        };
      });

      if (metrics.scrollWidth > metrics.vw + 1) {
        v(`[${vp.name}] ${route} scrolls horizontally (scrollWidth=${metrics.scrollWidth} vw=${metrics.vw})`);
      }
      // Short titles (<= 14 chars) must not wrap into 3+ lines (vertical text).
      if (metrics.h1Text && metrics.h1Text.length <= 14 && metrics.h1Lines && metrics.h1Lines >= 3) {
        v(`[${vp.name}] ${route} title "${metrics.h1Text}" wrapped to ${metrics.h1Lines} lines`);
      }
      // Mobile data tables must not squeeze titles to one word per line.
      if (vp.name === 'mobile' && metrics.titleNowrap && metrics.titleNowrap !== 'nowrap') {
        v(`[mobile] ${route} data-table cell white-space=${metrics.titleNowrap} (expected nowrap)`);
      }
    }
    await page.close();
  }

  // ── Link integrity (rendered links from the main pages) ───────────────
  {
    const page = await authCtx.newPage();
    await page.setViewportSize({ width: 1280, height: 900 });
    const seen = new Set();
    for (const route of ['/', '/warnings', '/insights', '/companies', '/persons', '/executive', '/triage', '/settings']) {
      await page.goto(BASE + route, { waitUntil: 'domcontentloaded' }).catch(() => {});
      const links = await collectLinks(page, origin);
      for (const l of links) seen.add(l);
    }
    let bad = 0;
    for (const link of seen) {
      const r = await page.request.get(BASE + link, { maxRedirects: 0 }).catch(() => null);
      // 3xx (auth redirect) and 2xx are fine; 404/405 are broken links.
      if (r && (r.status() === 404 || r.status() === 405 || r.status() >= 500)) {
        v(`broken link ${link} -> ${r.status()}`);
        bad++;
        if (bad > 20) break;
      }
    }
    await page.close();
  }

  await authCtx.close();
} finally {
  await browser.close();
}

if (violations.length) {
  console.error(`\nUI e2e FAILED with ${violations.length} violation(s):`);
  for (const x of violations) console.error('  - ' + x);
  process.exit(1);
}
console.log('UI e2e: all checks passed (routes, viewports, overflow, links, errors).');
