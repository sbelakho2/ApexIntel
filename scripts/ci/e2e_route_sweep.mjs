// Live GET-route sweep for the running ApexIntel API.
//
// Extracts every GET route registered in the Axum routers (app_router.rs and
// web/routes.rs), fills path parameters with the deterministic seed corpus
// (plus two seeded battlecards), and requests each one with an admin web
// session (web routes) or an admin API key (/api routes). The sweep fails on:
//   * any 5xx or transport error (503 is accepted only on the
//     capability-gated routes in DEGRADED_OK, whose honest answer without a
//     worker heartbeat / embedding provider is "unavailable"),
//   * a 404 for a route filled with a seeded id (a wired detail page that
//     cannot find its own fixture),
//   * a route whose path parameter this script does not know how to fill
//     (new routes must be added to PARAMS below, not silently skipped),
//   * a suspiciously small route extraction (the regex drifted from the code),
//   * ERROR-level lines the server logged during the sweep, when SERVER_LOG
//     points at the server's log file.
//
// Usage (repo root, server running, migrations applied):
//   BASE_URL=http://127.0.0.1:9095 ADMIN_USER=admin ADMIN_PASS=adminpassword \
//   SWEEP_ADMIN_KEY=<admin API key> DATABASE_URL=postgres://... \
//   [SERVER_LOG=/tmp/apex-api.log] node scripts/ci/e2e_route_sweep.mjs
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const { request } = require('@playwright/test');
const { Pool } = require('pg');
const { SEED, seedDatabase, databaseUrl } = require('../../e2e/helpers/server-ui-fixtures.cjs');

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const BASE = process.env.BASE_URL || 'http://127.0.0.1:9095';
const USER = process.env.ADMIN_USER || 'admin';
const PASS = process.env.ADMIN_PASS || 'adminpassword';
const KEY = process.env.SWEEP_ADMIN_KEY;
const SERVER_LOG = process.env.SERVER_LOG;
const MIN_ROUTES = 120;

if (!KEY) {
  console.error('SWEEP_ADMIN_KEY (an admin-role API key) is required');
  process.exit(2);
}

// ── route extraction ─────────────────────────────────────────────────────────

const ROUTER_FILES = ['crates/api/src/app_router.rs', 'crates/api/src/web/routes.rs'];

function extractGetRoutes() {
  const routes = new Set();
  for (const file of ROUTER_FILES) {
    const source = fs.readFileSync(path.join(ROOT, file), 'utf8').replace(/\s+/g, ' ');
    // `.route("/p", get(h))`, `.route("/p", get(h).post(h2))`, `.route("/p", post(h).get(h2))`
    for (const chunk of source.split(/\.route\( ?"/).slice(1)) {
      const match = chunk.match(/^([^"]+)" ?, ?(.*)$/);
      if (!match) continue;
      const handlerExpr = match[2].slice(0, 400).split('.route(')[0];
      if (/(^|[^_a-z])get\(/.test(handlerExpr)) routes.add(match[1]);
    }
    // Web builder helpers: `.get("/p", h)`, `.admin_get("/p", h)`, ...
    for (const match of source.matchAll(/\.(?:[a-z_]+_)?get\( ?"(\/[^"]*)"/g)) {
      routes.add(match[1]);
    }
  }
  return [...routes].sort();
}

// Streaming and service-worker endpoints never complete a plain GET.
const SKIP = new Set(['/api/v1/events/stream', '/sw.js']);

// ── fixtures ─────────────────────────────────────────────────────────────────

const BC1 = 'ba771e00-0000-4000-8000-000000000001';
const BC2 = 'ba771e00-0000-4000-8000-000000000002';
const COMPANY = SEED.companies[0].id;
const COMPANY2 = SEED.companies[1].id;
const PERSON = SEED.persons[0].id;
const WARNING = SEED.warnings[0].id;
const ANALYSIS_RUN = 'a7a1e000-0000-4000-8000-000000000001';
const MISSING = '00000000-0000-4000-8000-0000000000aa';
const SWEEP_EMBEDDING_MODEL = 'route-sweep-fixture';

// Sub-resources the base corpus does not create, so their detail routes
// render real rows instead of an expected 404. Idempotent.
async function seedSweepFixtures() {
  const pool = new Pool({ connectionString: databaseUrl() });
  try {
    await pool.query('DELETE FROM battlecards WHERE id = ANY($1::uuid[])', [[BC1, BC2]]);
    await pool.query(
      `INSERT INTO battlecards (id, our_company_id, competitor_id, title, status, positioning)
       VALUES ($1::uuid, $3::uuid, $4::uuid, 'Northwind vs Cobalt', 'draft', '"Uptime"'::jsonb),
              ($2::uuid, $4::uuid, $3::uuid, 'Cobalt vs Northwind', 'published', NULL)`,
      [BC1, BC2, COMPANY, COMPANY2],
    );
    await pool.query('DELETE FROM warning_analysis_runs WHERE id = $1::uuid', [ANALYSIS_RUN]);
    await pool.query(
      `INSERT INTO warning_analysis_runs
         (id, warning_id, status, requested_by, model, prompt_version, evidence_digest,
          observations_available, observations_sent, output, started_at, finished_at)
       VALUES ($1::uuid, $2::uuid, 'succeeded', 'route-sweep', 'route-sweep-model', 'v1',
               'route-sweep-digest', 2, 2, '{"summary":"Route sweep fixture analysis."}'::jsonb,
               NOW() - INTERVAL '2 minutes', NOW() - INTERVAL '1 minute')`,
      [ANALYSIS_RUN, WARNING],
    );
    await pool.query('DELETE FROM psychological_profiles WHERE person_id = $1', [PERSON]);
    await pool.query(
      `INSERT INTO psychological_profiles
         (person_id, decision_style, change_appetite, pain_index, risk_tolerance,
          preferred_proof, enrichment_quality, evidence_sources)
       VALUES ($1, 'analytical', 'moderate', 0.4, 0.6, ARRAY['case_study'],
               0.7, ARRAY['https://example.test/sweep/profile'])`,
      [PERSON],
    );
    await pool.query('DELETE FROM engagement_profiles WHERE person_id = $1', [PERSON]);
    await pool.query(
      `INSERT INTO engagement_profiles
         (person_id, talking_points, opening_topics, avoid_topics, best_channel, best_timing)
       VALUES ($1, ARRAY['Supply resilience'], ARRAY['Baltic routing'],
               ARRAY['Pricing'], 'email', 'Tuesday mornings')`,
      [PERSON],
    );
    await pool.query(
      `INSERT INTO entity_alert_configs (entity_id, config, updated_at)
       VALUES ($1, $2::jsonb, NOW())
       ON CONFLICT (entity_id) DO UPDATE SET config = EXCLUDED.config, updated_at = NOW()`,
      [
        COMPANY,
        JSON.stringify({
          entity_id: COMPANY,
          min_severity: 'medium',
          enabled_channels: ['in_app'],
          cooldown_minutes: 30,
          max_daily_alerts: 10,
          override_rules: [],
          enabled: true,
        }),
      ],
    );
    // Two nearby unit-ish vectors so /similar has a real neighbour.
    await pool.query('DELETE FROM embeddings WHERE model_name = $1', [SWEEP_EMBEDDING_MODEL]);
    const dims = (
      await pool.query(
        `SELECT atttypmod AS dims FROM pg_attribute
         WHERE attrelid = 'embeddings'::regclass AND attname = 'embedding'`,
      )
    ).rows[0].dims;
    for (const [id, value] of [[COMPANY, 0.05], [COMPANY2, 0.051]]) {
      await pool.query(
        `INSERT INTO embeddings (entity_type, entity_id, chunk_index, embedding, source_text, model_name)
         VALUES ('company', $1, 0, array_fill($2::real, ARRAY[$3::int])::vector, 'route sweep fixture', $4)`,
        [id, value, dims, SWEEP_EMBEDDING_MODEL],
      );
    }
  } finally {
    await pool.end();
  }
}

// `:id` is resolved by the first path segment after an optional `/api`.
const ID_BY_ROOT = {
  battlecards: BC1,
  companies: COMPANY,
  competitors: COMPANY,
  persons: PERSON,
  warnings: WARNING,
  insights: SEED.insight.id,
  workspaces: SEED.workspace.id,
  triage: SEED.triageMerge.id,
  pipeline: SEED.pipeline.id,
  charts: COMPANY,
  graph: COMPANY,
  entities: COMPANY,
  icp: COMPANY,
};

const PARAMS = {
  entity_type: () => 'company',
  entity_id: () => COMPANY,
  from: () => COMPANY,
  to: () => COMPANY2,
  run_id: () => ANALYSIS_RUN,
  user_id: () => USER,
  id: (root) => ID_BY_ROOT[root] || MISSING,
};

// Routes whose `:id` is created through the real POST endpoint at sweep
// start (filled in by createViaApi), so create -> read is exercised live.
const ID_BY_ROUTE = {};
const API_CREATED_TITLE = 'Route sweep fixture';

const SEEDED_IDS = new Set([BC1, BC2, ...Object.values(ID_BY_ROOT), COMPANY2, ANALYSIS_RUN]);

// Capability-gated routes: without a live worker heartbeat or embedding
// provider (CI has neither) their correct answer is 503. Any other 5xx on
// them, and any 5xx elsewhere, still fails.
const DEGRADED_OK = new Set([
  '/api/health/ready',
  '/api/search/vector',
  '/data/healthy',
  '/intelligence/healthy',
  '/process/ready',
]);

const QUERY = {
  '/battlecards/compare': `?ids=${BC1},${BC2}`,
  '/battlecards/compare/export': `?ids=${BC1}&ids=${BC2}`,
  '/search': '?q=northwind',
  '/search/suggestions': '?q=nor',
  '/api/search': '?q=northwind',
  '/api/search/semantic': '?q=northwind',
  '/api/search/suggest': '?q=nor',
  '/api/search/vector': '?q=northwind',
  '/api/trends/comparison': (() => {
    const day = (offset) => new Date(Date.now() - offset * 86400000).toISOString().slice(0, 10);
    return `?metric=warnings&current_start=${day(30)}&previous_start=${day(60)}&days=30`;
  })(),
  '/api/trends/entities': `?entity_type=company&entity_id=${COMPANY}&metric=warnings&bucket=daily`,
};

// Pagination / filter variants of list pages that the bare route does not hit.
const EXTRA_URLS = [
  '/companies?page=2',
  '/persons?page=2',
  '/insights?page=2',
  '/warnings?page=2',
  '/warnings?severity=critical',
  '/pipeline?stage=discovery',
  '/supplier-risk?category=financial',
];

function fill(route) {
  const segments = route.split('/').filter(Boolean);
  const root = segments[0] === 'api' ? segments[1] : segments[0];
  const unknown = [];
  const url = route.replace(/:([a-z_]+)/g, (_, name) => {
    if (name === 'id' && ID_BY_ROUTE[route]) return ID_BY_ROUTE[route];
    const resolve = PARAMS[name];
    if (!resolve) {
      unknown.push(name);
      return `:${name}`;
    }
    return resolve(root);
  });
  return { url, unknown };
}

// ── sweep ────────────────────────────────────────────────────────────────────

const failures = [];
const routes = extractGetRoutes();
if (routes.length < MIN_ROUTES) {
  failures.push(`route extraction found only ${routes.length} GET routes (< ${MIN_ROUTES}); update the extractor`);
}

for (const route of DEGRADED_OK) {
  if (!routes.includes(route)) failures.push(`DEGRADED_OK lists ${route}, which is no longer a GET route`);
}

await seedDatabase();
await seedSweepFixtures();

const logOffset = SERVER_LOG && fs.existsSync(SERVER_LOG) ? fs.statSync(SERVER_LOG).size : 0;

const web = await request.newContext({ baseURL: BASE });
const login = await web.post('/login', { form: { username: USER, password: PASS }, maxRedirects: 0 });
if (![302, 303].includes(login.status())) {
  console.error(`login failed: HTTP ${login.status()}`);
  process.exit(1);
}
const api = await request.newContext({
  baseURL: BASE,
  extraHTTPHeaders: { Authorization: `Bearer ${KEY}` },
});

async function createViaApi(route, collectionPath, body) {
  const response = await api.post(collectionPath, { data: { title: API_CREATED_TITLE, ...body } });
  const payload = await response.json().catch(() => null);
  const id = payload?.data?.id;
  if (response.status() !== 200 || !id) {
    failures.push(`POST ${collectionPath} -> ${response.status()} :: ${JSON.stringify(payload).slice(0, 200)}`);
    return;
  }
  if (payload.data.created_by !== USER) {
    failures.push(`POST ${collectionPath} recorded created_by=${payload.data.created_by}, expected ${USER}`);
  }
  ID_BY_ROUTE[route] = id;
  SEEDED_IDS.add(id);
}

async function deleteApiCreatedFixtures() {
  const pool = new Pool({ connectionString: databaseUrl() });
  try {
    await pool.query('DELETE FROM strategic_opportunities WHERE title = $1', [API_CREATED_TITLE]);
    await pool.query('DELETE FROM critical_threats WHERE title = $1', [API_CREATED_TITLE]);
  } finally {
    await pool.end();
  }
}

async function hit(url, isApi, degradedOk = false) {
  let response;
  try {
    response = await (isApi ? api : web).get(url, { maxRedirects: 0, timeout: 30000 });
  } catch (error) {
    failures.push(`GET ${url} transport error: ${String(error.message).split('\n')[0]}`);
    return;
  }
  const code = response.status();
  console.log(`${code} ${url}`);
  const seededTarget = [...SEEDED_IDS].some((id) => url.includes(id));
  const acceptedDegraded = code === 503 && degradedOk;
  if ((code >= 500 && !acceptedDegraded) || (code === 404 && seededTarget)) {
    const body = (await response.text()).slice(0, 200).replace(/\s+/g, ' ');
    failures.push(`GET ${url} -> ${code} :: ${body}`);
  }
}

await deleteApiCreatedFixtures();
await createViaApi('/api/executive/opportunities/:id', '/api/executive/opportunities', {
  opportunity_type: 'market_expansion',
  priority_score: 0.7,
  confidence: 0.6,
  entity_id: COMPANY,
  entity_type: 'company',
  recommended_actions: ['Brief the account team'],
});
await createViaApi('/api/executive/threats/:id', '/api/executive/threats', {
  threat_type: 'supply_chain',
  severity: 'high',
  impact_score: 0.8,
  confidence: 0.7,
  mitigation_steps: ['Qualify a second supplier'],
});

let swept = 0;
for (const route of routes) {
  if (SKIP.has(route)) continue;
  const { url, unknown } = fill(route);
  if (unknown.length > 0) {
    failures.push(`${route}: no fixture for path parameter(s) ${unknown.join(', ')}`);
    continue;
  }
  await hit(url + (QUERY[route] || ''), route.startsWith('/api/'), DEGRADED_OK.has(route));
  swept += 1;
}
for (const url of EXTRA_URLS) {
  await hit(url, false);
  swept += 1;
}

await web.dispose();
await api.dispose();

if (SERVER_LOG) {
  if (!fs.existsSync(SERVER_LOG)) {
    failures.push(`SERVER_LOG ${SERVER_LOG} does not exist`);
  } else {
    const fd = fs.openSync(SERVER_LOG, 'r');
    const length = fs.statSync(SERVER_LOG).size - logOffset;
    const buffer = Buffer.alloc(Math.max(length, 0));
    fs.readSync(fd, buffer, 0, buffer.length, logOffset);
    fs.closeSync(fd);
    const errorLines = buffer
      .toString('utf8')
      .split('\n')
      .filter((line) => /\bERROR\b/.test(line));
    for (const line of errorLines.slice(0, 20)) failures.push(`server logged: ${line.slice(0, 300)}`);
    if (errorLines.length > 20) failures.push(`... and ${errorLines.length - 20} more ERROR lines`);
  }
}

console.log(`\nswept ${swept} URLs (${routes.length} GET routes extracted)`);
if (failures.length > 0) {
  console.error(`\nroute sweep FAILED (${failures.length}):`);
  for (const failure of failures) console.error(`  ${failure}`);
  process.exit(1);
}
console.log('route sweep passed');
