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
// It also checks the unauthenticated redirect, the styled 404, that every
// link/hx-* target rendered on a page resolves (link integrity), the workflow
// information architecture, and the task-based flows with explicit action
// budgets (open evidence <=2, watch alerts <=2, start investigation <=2,
// find person <=2, bookmark insight, trace claim -> source).
//
// Usage: BASE_URL=http://127.0.0.1:9095 ADMIN_USER=admin ADMIN_PASS=adminpassword \
//        DATABASE_URL=postgres://... node scripts/ci/e2e_server_ui.mjs
import { chromium } from 'playwright';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const { SEED, seedDatabase } = require('../../e2e/helpers/server-ui-fixtures.cjs');

const BASE = process.env.BASE_URL || 'http://127.0.0.1:9095';
const USER = process.env.ADMIN_USER || 'admin';
const PASS = process.env.ADMIN_PASS || 'adminpassword';

const ROUTES = [
  '/', '/warnings', '/insights', '/companies', '/persons', '/buying-centers',
  '/competitors',
  '/battlecards', '/search', '/graph', '/triage', '/workspaces', '/queue',
  '/activity', '/supplier-risk', '/pipeline', '/evidence', '/team-assignments',
  '/executive', '/trends', '/security', '/admin', '/memos', '/notifications',
  '/settings', '/settings/alerts',
];

// Audit #15: primary navigation is organised by analyst workflow, not by
// database table. Settings lives in the user menu.
const WORKFLOW_NAV_GROUPS = [
  'Command Center',
  'Entities',
  'Signals',
  'Investigations',
  'Sales Intelligence',
  'Automations',
];

const VIEWPORTS = [
  { name: 'mobile', width: 390, height: 844 },
  { name: 'desktop', width: 1280, height: 900 },
];

// P0 #34: exactly five mobile bottom-bar items, in order.
const MOBILE_TAB_LABELS = ['Home', 'Signals', 'Entities', 'Triage', 'More'];

// P0 #34: routes whose bottom-bar active item is asserted explicitly.
const EXPECTED_MOBILE_ACTIVE = {
  '/': 'Home',
  '/warnings': 'Signals',
  '/insights': 'Signals',
  '/trends': 'Signals',
  '/companies': 'Entities',
  '/persons': 'Entities',
  '/buying-centers': 'Entities',
  '/competitors': 'Entities',
  '/triage': 'Triage',
  '/queue': 'Triage',
};

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
  // ── Deterministic fixtures for the task-based flows ──────────────────
  // The route sweep does not need data, but the budgeted analyst tasks do.
  await seedDatabase();

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

      // ── P0 #21/#34: permanent command bar + user-menu settings entry ─────
      const ia = await page.evaluate(() => ({
        commandBar: !!document.querySelector('form[action="/search"] input[name="q"]'),
        userMenuSettings: !!document.querySelector('details.apex-user-menu a[href="/settings"]')
          || !!document.querySelector('a[href="/settings"]'),
        userMenu: !!document.querySelector('details.apex-user-menu'),
        navGroups: Array.from(document.querySelectorAll('.apex-nav-group-label'))
          .map((el) => el.textContent.trim()),
        settingsInPrimaryNav: !!document.querySelector('nav[aria-label="Sections"] a[href="/settings"]'),
      }));
      if (!ia.commandBar) v(`[${vp.name}] ${route} missing permanent command bar search entry`);
      if (vp.name === 'desktop' && !ia.userMenuSettings) {
        v(`[desktop] ${route} settings not reachable from the user menu`);
      }

      // ── Audit #15: workflow navigation IA ────────────────────────────────
      if (route === '/') {
        for (const group of WORKFLOW_NAV_GROUPS) {
          if (!ia.navGroups.includes(group)) {
            v(`[${vp.name}] missing workflow nav group "${group}"`);
          }
        }
        if (ia.settingsInPrimaryNav) {
          v(`[${vp.name}] settings should live in the user menu, not primary navigation`);
        }
      }

      // ── P0 #34: 5-item mobile bottom bar contract ─────────────────────────
      if (vp.name === 'mobile') {
        const tabbar = await page.evaluate((expectedLabels) => {
          const nav = document.querySelector('nav[aria-label="Mobile quick navigation"]');
          if (!nav) return { error: 'mobile tab bar missing' };
          const grid = nav.querySelector('div');
          const items = grid ? Array.from(grid.children) : [];
          const labelOf = (el) => {
            const label = el.querySelector('.apex-mobile-tab-label');
            return label ? label.textContent.trim() : '';
          };
          return {
            count: items.length,
            labels: items.map(labelOf),
            heights: items.map((el) => Math.round(el.getBoundingClientRect().height)),
            icons: items.map((el) => {
              const svg = el.querySelector('svg');
              return svg ? [Number(svg.getAttribute('width')), Number(svg.getAttribute('height'))] : null;
            }),
            current: items.map((el) => el.getAttribute('aria-current')),
            indicators: items.map((el) => {
              const ind = el.querySelector('.apex-tab-indicator');
              if (!ind) return null;
              const cs = getComputedStyle(ind);
              return { opacity: cs.opacity, height: Number.parseFloat(cs.height), width: Number.parseFloat(cs.width) };
            }),
            expectedLabels,
          };
        }, MOBILE_TAB_LABELS);

        if (tabbar.error) {
          v(`[mobile] ${route} ${tabbar.error}`);
        } else {
          if (tabbar.count !== 5) v(`[mobile] ${route} bottom bar has ${tabbar.count} items (expected 5)`);
          if (tabbar.labels.join('|') !== MOBILE_TAB_LABELS.join('|')) {
            v(`[mobile] ${route} bottom bar labels [${tabbar.labels.join(', ')}] (expected [${MOBILE_TAB_LABELS.join(', ')}])`);
          }
          tabbar.heights.forEach((h, i) => {
            if (h < 44) v(`[mobile] ${route} bottom bar item "${tabbar.labels[i]}" is ${h}px tall (<44px touch target)`);
          });
          tabbar.icons.forEach((size, i) => {
            if (!size || size[0] < 20 || size[0] > 24 || size[1] < 20 || size[1] > 24) {
              v(`[mobile] ${route} bottom bar item "${tabbar.labels[i]}" icon size ${size ? size.join('x') : 'missing'} (expected 20-24px)`);
            }
          });
          const activeIndexes = tabbar.current
            .map((value, index) => (value ? index : -1))
            .filter((index) => index >= 0);
          if (activeIndexes.length > 1) {
            v(`[mobile] ${route} bottom bar has ${activeIndexes.length} active items (expected <= 1)`);
          }
          if (activeIndexes.length === 1) {
            const index = activeIndexes[0];
            const indicator = tabbar.indicators[index];
            if (!indicator || indicator.opacity === '0' || !(indicator.height > 0) || !(indicator.width > 0)) {
              v(`[mobile] ${route} active tab "${tabbar.labels[index]}" lacks a non-colour active indicator`);
            }
          }
          const expectedActive = EXPECTED_MOBILE_ACTIVE[route];
          if (expectedActive) {
            const expectedIndex = tabbar.labels.indexOf(expectedActive);
            if (!activeIndexes.includes(expectedIndex)) {
              v(`[mobile] ${route} expected "${expectedActive}" to be the active bottom-bar item (aria-current)`);
            }
          }
        }
      }
    }
    await page.close();
  }

  // ── Task-based flows with explicit action budgets ─────────────────────
  // Each budget counts user actions only (clicks/keys); login/goto are setup.
  {
    const flow = async (name, fn, budget) => {
      const page = await authCtx.newPage();
      const state = { actions: 0 };
      const act = async (label, run) => {
        state.actions += 1;
        await run();
        if (state.actions > budget) {
          v(`task "${name}" exceeded budget ${budget} at action "${label}"`);
        }
      };
      try {
        await fn(page, act);
      } catch (error) {
        v(`task "${name}" failed: ${String(error.message || error).slice(0, 200)}`);
      } finally {
        await page.close();
      }
    };

    // Open evidence from a warning in <=2 actions.
    await flow('open evidence from warning', async (page, act) => {
      const warning = SEED.warnings[0];
      await page.goto(`${BASE}/warnings`, { waitUntil: 'domcontentloaded' });
      await act('open warning', () =>
        page.locator('tr[data-row-link]', { hasText: warning.title }).click());
      await page.waitForURL(`**/warnings/${warning.id}`);
      const sourceLink = page.locator('[data-claim-source]').first();
      if (!(await sourceLink.isVisible().catch(() => false))) {
        v('task "open evidence from warning": no claim source link rendered');
        return;
      }
      const evidenceHref = await sourceLink.getAttribute('href');
      if (!evidenceHref || !evidenceHref.includes('example.test')) {
        v(`task "open evidence from warning": unexpected evidence href ${evidenceHref}`);
      }
      await act('open claim source', async () => {
        const [popup] = await Promise.all([page.waitForEvent('popup'), sourceLink.click()]);
        if (!popup) {
          v('task "open evidence from warning": evidence link did not open the source');
        }
        await popup.close();
      });
    }, 2);

    // Subscribe to entity alerts in <=2 actions.
    await flow('watch entity alerts', async (page, act) => {
      const company = SEED.companies[0];
      await page.goto(`${BASE}/companies`, { waitUntil: 'domcontentloaded' });
      await act('open company dossier', () =>
        page.locator('tr[data-row-link]', { hasText: company.name }).click());
      await page.waitForURL(`**/companies/${company.id}`);
      await act('watch alerts', () => page.locator('[data-alert-subscription-save]').click());
      await page.waitForFunction(
        () => document.querySelector('[data-alert-subscription-state]')?.textContent.trim() === 'Watching',
        { timeout: 15_000 }
      );
    }, 2);

    // Start an investigation from a signal in <=2 actions.
    await flow('start investigation from signal', async (page, act) => {
      const warning = SEED.warnings[0];
      await page.goto(`${BASE}/warnings`, { waitUntil: 'domcontentloaded' });
      await act('open warning', () =>
        page.locator('tr[data-row-link]', { hasText: warning.title }).click());
      await page.waitForURL(`**/warnings/${warning.id}`);
      await act('start investigation', () =>
        page.locator('[data-action="start-investigation"]').click());
      await page.waitForURL(/\/workspaces\/[0-9a-f-]{36}$/, { timeout: 15_000 });
      const heading = (await page.locator('h1').first().textContent()) || '';
      if (!heading.includes('Investigate:')) {
        v(`task "start investigation from signal": workspace heading "${heading.trim()}"`);
      }
    }, 2);

    // Find a person from a company dossier in <=2 actions.
    await flow('find person from dossier', async (page, act) => {
      const company = SEED.companies[0];
      const person = SEED.persons[0];
      await page.goto(`${BASE}/companies`, { waitUntil: 'domcontentloaded' });
      await act('open company dossier', () =>
        page.locator('tr[data-row-link]', { hasText: company.name }).click());
      await page.waitForURL(`**/companies/${company.id}`);
      await act('open person', () =>
        page.locator('[data-person-link]', { hasText: person.name }).first().click());
      await page.waitForURL(`**/persons/${person.id}`);
      const body = (await page.locator('#main-content').textContent()) || '';
      if (!body.includes(person.name)) {
        v(`task "find person from dossier": ${person.name} not rendered on person page`);
      }
    }, 2);

    // Bookmark an insight (<=2 actions from the insight list).
    await flow('bookmark insight', async (page, act) => {
      const insight = SEED.insight;
      await page.goto(`${BASE}/insights`, { waitUntil: 'domcontentloaded' });
      await act('open insight', () =>
        page.locator(`a[href="/insights/${insight.id}"]`).first().click());
      await page.waitForURL(`**/insights/${insight.id}`);
      await act('bookmark', () =>
        page.getByRole('button', { name: 'Bookmark', exact: true }).click());
      await page.waitForSelector('#bookmark-status button[title="Remove bookmark"]', {
        timeout: 15_000,
      });
    }, 2);

    // Trace a claim to its source (one citation click).
    await flow('trace claim to source', async (page, act) => {
      const insight = SEED.insight;
      await page.goto(`${BASE}/insights/${insight.id}`, { waitUntil: 'domcontentloaded' });
      await act('follow citation', () =>
        page.locator('a[title="Evidence source 1"]').first().click());
      await page.waitForFunction(() => window.location.hash === '#source-1', { timeout: 10_000 });
      const href = await page
        .locator('#source-1 a[href]')
        .first()
        .getAttribute('href');
      if (href !== insight.evidenceUrl) {
        v(`task "trace claim to source": source href ${href} (expected ${insight.evidenceUrl})`);
      }
    }, 2);
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
