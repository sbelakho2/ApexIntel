// @ts-check
// Journey-contract e2e for the server-rendered UI.
//
// Each test drives the real Askama/HTMX UI the way an analyst does, then
// asserts the PostgreSQL rows the action must have produced. Rendering success
// is never the assertion: a route that returns 200 but writes nothing fails
// here. Run against a live `apex-api` (see playwright.server-ui.config.cjs):
//
//   BASE_URL=http://127.0.0.1:9095 \
//   DATABASE_URL=postgres://... \
//     npx playwright test -c playwright.server-ui.config.cjs e2e/server-ui-journeys.spec.js
//
// The route sweep stays in scripts/ci/e2e_server_ui.mjs (smoke stage); this
// file is the journey stage.
const { test, expect } = require('@playwright/test');
const { Pool } = require('pg');
const { SEED, seedDatabase, login, databaseUrl } = require('./helpers/server-ui-fixtures.cjs');

const JOURNEY = {
  battlecardId: 'ba771e00-0000-4000-8000-000000000001',
  graphEdgeId: '6ea00000-0000-4000-8000-000000000001',
  createdWorkspaceName: 'Journey Workspace — Capacity Review',
  savedSearchName: 'Journey saved search',
  recipeName: 'Journey Recipe — Capacity Surge',
};

let pool;

async function q(sql, params = []) {
  const result = await pool.query(sql, params);
  return result.rows;
}

async function scalar(sql, params = []) {
  const rows = await q(sql, params);
  return rows.length ? Object.values(rows[0])[0] : undefined;
}

/** Poll a DB predicate; keeps tests deterministic without render-only flakiness. */
async function expectDb(fn, expected, timeout = 15_000) {
  await expect.poll(fn, { timeout }).toBe(expected);
}

/** The first entity id referenced by a seed warning's company. */
const COMPANY_ID = SEED.companies[0].id;
const PERSON_ID = SEED.persons[0].id;

async function seedJourneyExtras() {
  // Battlecard for the regenerate journey (regenerated state is reset so the
  // assertion proves the click, not the seed).
  await q('DELETE FROM battlecards WHERE id = $1::uuid', [JOURNEY.battlecardId]);
  await q(
    `INSERT INTO battlecards (id, our_company_id, competitor_id, title, status)
     VALUES ($1::uuid, $2::uuid, $3::uuid, 'Northwind vs Cobalt', 'draft')`,
    [JOURNEY.battlecardId, COMPANY_ID, SEED.companies[1].id]
  );

  // Graph edge for the traversal journey: the neighborhood endpoint reads this.
  await q('DELETE FROM graph_edges WHERE id = $1::uuid', [JOURNEY.graphEdgeId]);
  await q(
    `INSERT INTO graph_edges
       (id, source_id, source_type, target_id, target_type, edge_type, weight, confidence)
     VALUES ($1::uuid, $2::uuid, 'company', $3::uuid, 'person', 'associated_with', 1.0, 0.9)`,
    [JOURNEY.graphEdgeId, COMPANY_ID, PERSON_ID]
  );

  // NOTE: `worker_trigger_queue` is intentionally NOT cleared. The trigger
  // journeys assert that the UI/API leaves a queued (unclaimed) job of the
  // requested kind, which holds whether the queue inserted a new row or
  // deduplicated an existing one. Never wipe a real queue to make a test
  // deterministic.
}

/**
 * Refuse to mutate a database that is not obviously disposable. The suite
 * seeds and deletes rows (battlecards, graph edges, sessions, preferences),
 * so pointing it at a shared/production database is a data-loss event, not a
 * test failure.
 */
function assertDisposableDatabase(url) {
  const parsed = new URL(url);
  const database = parsed.pathname.replace(/^\//, '');
  const host = parsed.hostname;
  const isLocal =
    host === 'localhost' ||
    host === '127.0.0.1' ||
    host === '::1' ||
    host === '[::1]' ||
    host === 'database';
  const namedDisposable = /(_ci|_test)$/.test(database);
  if (!isLocal && !namedDisposable && process.env.APEX_E2E_ALLOW_NON_DISPOSABLE !== '1') {
    throw new Error(
      `refusing to run destructive journey e2e against '${database}' on '${host}': ` +
        'use a local database or a name ending in _ci/_test, or set ' +
        'APEX_E2E_ALLOW_NON_DISPOSABLE=1 if you really intend to mutate it.'
    );
  }
}

test.beforeAll(async () => {
  assertDisposableDatabase(databaseUrl());
  await seedDatabase();
  pool = new Pool({ connectionString: databaseUrl(), max: 4 });
  await q('SELECT 1');
});

test.beforeEach(async () => {
  await seedDatabase();
  await seedJourneyExtras();
});

test.afterAll(async () => {
  if (pool) await pool.end();
});

test.describe('server UI — journey contracts (DB state)', () => {
  test('creates a workspace and persists it', async ({ page }) => {
    await login(page);
    await q('DELETE FROM investigation_workspaces WHERE name = $1', [
      JOURNEY.createdWorkspaceName,
    ]);

    await page.goto('/workspaces/new', { waitUntil: 'domcontentloaded' });
    await page.fill('input[name="name"]', JOURNEY.createdWorkspaceName);
    await page.fill('textarea[name="description"]', 'Journey-created workspace.');
    await page.selectOption('select[name="workspace_type"]', 'structured');
    await page.fill('input[name="tags"]', 'journey, supply');
    await Promise.all([
      page.waitForURL(/\/workspaces\/[0-9a-f-]{36}$/),
      page.getByRole('button', { name: 'Create Workspace' }).click(),
    ]);

    const workspaceId = page.url().match(/workspaces\/([0-9a-f-]{36})/)[1];
    const rows = await q(
      `SELECT name, workspace_type, owner_id, status, visibility, tags, entity_focus
         FROM investigation_workspaces WHERE id = $1::uuid`,
      [workspaceId]
    );
    expect(rows).toHaveLength(1);
    expect(rows[0].name).toBe(JOURNEY.createdWorkspaceName);
    expect(rows[0].workspace_type).toBe('structured');
    expect(rows[0].owner_id).toBe('admin');
    expect(rows[0].status).toBe('active');
    expect(rows[0].visibility).toBe('team');
    expect(rows[0].tags).toEqual(['journey', 'supply']);
    expect(rows[0].entity_focus).toEqual([]);

    await q('DELETE FROM investigation_workspaces WHERE id = $1::uuid', [workspaceId]);
  });

  test('saves a search and deletes it', async ({ page }) => {
    await login(page);
    await q('DELETE FROM saved_searches WHERE user_id = $1 AND name = $2', [
      'admin',
      JOURNEY.savedSearchName,
    ]);

    await page.goto('/search?q=Northwind', { waitUntil: 'domcontentloaded' });
    await page.fill('input[name="name"]', JOURNEY.savedSearchName);
    await page.getByRole('button', { name: 'Save current search' }).click();

    let savedId;
    await expect
      .poll(async () => {
        const rows = await q(
          'SELECT id, query_text, filters FROM saved_searches WHERE user_id = $1 AND name = $2',
          ['admin', JOURNEY.savedSearchName]
        );
        savedId = rows[0]?.id;
        return rows.length;
      })
      .toBe(1);
    const saved = (await q('SELECT query_text, filters FROM saved_searches WHERE id = $1::uuid', [
      savedId,
    ]))[0];
    expect(saved.query_text).toBe('Northwind');
    expect(saved.filters.entity_type).toBe('all');

    await page.goto('/search?q=Northwind', { waitUntil: 'domcontentloaded' });
    await page.click(`form[action="/search/saved-searches/${savedId}/delete"] button`);
    await expectDb(
      async () => scalar('SELECT count(*)::int FROM saved_searches WHERE id = $1::uuid', [savedId]),
      0
    );
  });

  test('acknowledges a warning and persists the acknowledgement', async ({ page }) => {
    await login(page);
    const warning = SEED.warnings[0];

    await page.goto(`/warnings/${warning.id}`, { waitUntil: 'domcontentloaded' });
    await page.getByRole('button', { name: 'Acknowledge', exact: true }).click();
    await expect(page.locator('#ack-status')).toContainText(/acknowledged/i, { timeout: 15_000 });

    const row = (
      await q(
        `SELECT acknowledged, acknowledged_by, acknowledged_at
           FROM warnings WHERE id = $1::uuid`,
        [warning.id]
      )
    )[0];
    expect(row.acknowledged).toBe(true);
    expect(row.acknowledged_by).toBe('admin');
    expect(row.acknowledged_at).not.toBeNull();
  });

  test('records a review outcome (true_positive) on a warning', async ({ page }) => {
    await login(page);
    const warning = SEED.warnings[0];

    await page.goto(`/warnings/${warning.id}`, { waitUntil: 'domcontentloaded' });
    await page.selectOption('select[name="review_outcome"]', 'true_positive');
    await page.fill('textarea[name="note"]', 'Confirmed by journey test.');
    await page.getByRole('button', { name: 'Save Review' }).click();

    await expectDb(
      async () =>
        scalar(
          'SELECT review_outcome FROM warnings WHERE id = $1::uuid',
          [warning.id]
        ),
      'true_positive'
    );
    const row = (
      await q(
        'SELECT reviewed_by, reviewed_at, acknowledged FROM warnings WHERE id = $1::uuid',
        [warning.id]
      )
    )[0];
    expect(row.reviewed_by).toBe('admin');
    expect(row.reviewed_at).not.toBeNull();
    expect(row.acknowledged).toBe(true);
  });

  // Warnings expose Acknowledge + Save Review (true/false positive); the
  // resolve/dismiss lifecycle lives on the warning's triage-queue item, which
  // is seeded as a mirror of warning #1 (seed row 7e570000-…).
  test('resolves a triage item and persists the status', async ({ page }) => {
    await login(page);
    page.on('dialog', (dialog) => dialog.accept());

    await page.goto(`/triage/${SEED.triageMerge.id}`, { waitUntil: 'domcontentloaded' });
    await page.getByRole('button', { name: 'Resolve', exact: true }).click();
    await expectDb(
      async () => scalar('SELECT status FROM triage_queue WHERE id = $1::uuid', [SEED.triageMerge.id]),
      'resolved'
    );
  });

  test('dismisses a triage item and persists the status', async ({ page }) => {
    await login(page);
    page.on('dialog', (dialog) => dialog.accept());

    await page.goto(`/triage/${SEED.triageMerge.id}`, { waitUntil: 'domcontentloaded' });
    await page.getByRole('button', { name: 'Dismiss', exact: true }).click();
    await expectDb(
      async () => scalar('SELECT status FROM triage_queue WHERE id = $1::uuid', [SEED.triageMerge.id]),
      'dismissed'
    );
  });

  test('bookmarks an insight (and unbookmarks it again)', async ({ page }) => {
    await login(page);
    const insight = SEED.insight;

    // Re-entrant: a retry after a partially completed toggle must start from a
    // clean state, otherwise the toggle click would unbookmark immediately and
    // the flow below could never observe its own writes.
    await q(
      'DELETE FROM insight_bookmarks WHERE insight_id = $1::uuid AND user_id = $2',
      [insight.id, 'admin']
    );

    const bookmarkCount = () =>
      scalar(
        'SELECT count(*)::int FROM insight_bookmarks WHERE insight_id = $1::uuid AND user_id = $2',
        [insight.id, 'admin']
      );

    // The bookmark control is a self-replacing text button in the page header:
    // one click always yields the opposite action in place.
    const removeButton = page.getByRole('button', { name: 'Unbookmark', exact: true });
    const bookmarkButton = page.getByRole('button', { name: 'Bookmark', exact: true });

    // htmx can drop a click that lands inside a response-swap window. Retry the
    // click until the server *response* arrives (never fire a second click
    // while one is in flight, or the toggle would flip twice); then assert the
    // persisted row, which is the actual workflow contract.
    const clickUntilResponse = async (clicker) => {
      const response = page
        .waitForResponse((r) => r.url().includes('/bookmark'), { timeout: 5_000 })
        .catch(() => null);
      await clicker();
      const resp = await response;
      if (resp) {
        await resp.finished();
      }
    };

    await page.goto(`/insights/${insight.id}`, { waitUntil: 'domcontentloaded' });
    await expect
      .poll(
        async () => {
          if ((await removeButton.count()) === 0) {
            await clickUntilResponse(() =>
              page.getByRole('button', { name: 'Bookmark', exact: true }).click()
            );
          }
          return removeButton.count();
        },
        { timeout: 60_000 }
      )
      .toBe(1);
    expect(await bookmarkCount()).toBe(1);

    await expect
      .poll(
        async () => {
          if ((await removeButton.count()) > 0) {
            await clickUntilResponse(() => removeButton.click());
          }
          return bookmarkButton.count();
        },
        { timeout: 60_000 }
      )
      .toBe(1);
    expect(await bookmarkCount()).toBe(0);
  });

  test('creates a recipe and stores it as staging', async ({ page }) => {
    await login(page);
    await q('DELETE FROM recipes WHERE name = $1', [JOURNEY.recipeName]);

    await page.goto('/recipes/new', { waitUntil: 'domcontentloaded' });
    await page.fill('input[name="name"]', JOURNEY.recipeName);
    await page.selectOption('select[name="severity"]', 'high');
    await page.fill('textarea[name="description"]', 'Journey-created recipe.');
    // The engine cannot evaluate a recipe without a signal; the documented
    // contract is "fill at least one", so the journey supplies a real one.
    await page.locator('input[name="signal_observation"]').first().fill('NewsArticle');
    await page.locator('input[name="signal_field"]').first().fill('count');
    await page.locator('input[name="signal_threshold"]').first().fill('3');
    await page.locator('input[name="signal_window_days"]').first().fill('30');
    await page.getByRole('button', { name: 'Create Recipe' }).click();
    await page.waitForURL(/\/recipes\?created=1/, { timeout: 15_000 });

    const row = (
      await q('SELECT code, status, definition FROM recipes WHERE name = $1', [JOURNEY.recipeName])
    )[0];
    expect(row).toBeTruthy();
    expect(row.status).toBe('staging');
    expect(row.definition.severity).toBe('high');
    expect(row.definition.description).toBe('Journey-created recipe.');
    expect(row.code).toMatch(/^[a-z0-9_]+_[0-9a-f]{8}$/);
  });

  test('runs an investigation from a warning and persists the workspace', async ({ page }) => {
    await login(page);
    const warning = SEED.warnings[0];

    await page.goto(`/warnings/${warning.id}`, { waitUntil: 'domcontentloaded' });
    await page.locator('[data-action="start-investigation"]').click();
    await page.waitForURL(/\/workspaces\/[0-9a-f-]{36}$/, { timeout: 15_000 });

    const workspaceId = page.url().match(/workspaces\/([0-9a-f-]{36})/)[1];
    const row = (
      await q(
        `SELECT name, workspace_type, owner_id, status, entity_focus
           FROM investigation_workspaces WHERE id = $1::uuid`,
        [workspaceId]
      )
    )[0];
    expect(row.name).toBe(`Investigate: ${warning.title}`);
    expect(row.workspace_type).toBe('incident');
    expect(row.owner_id).toBe('admin');
    expect(row.status).toBe('active');
    expect(row.entity_focus).toEqual([COMPANY_ID]);
  });

  test('exports insights CSV and records the export', async ({ page }) => {
    await login(page);
    const startedAt = new Date();

    await page.goto('/insights', { waitUntil: 'domcontentloaded' });
    await expect(page.locator('a[href="/api/insights/export"]')).toBeVisible();

    const response = await page.request.get('/api/insights/export');
    expect(response.ok()).toBeTruthy();
    expect(response.headers()['content-type']).toContain('text/csv');
    const body = await response.text();
    expect(body.split('\n')[0]).toContain('id,');
    expect(body).toContain(SEED.insight.title);

    await expect.poll(
      async () =>
        scalar(
          `SELECT count(*)::int FROM export_history
             WHERE user_id = 'admin' AND export_type = 'insights'
               AND format = 'csv' AND download_name = 'insights.csv'
               AND requested_at >= $1`,
          [startedAt.toISOString()]
        ),
      { timeout: 15_000 }
    ).toBeGreaterThanOrEqual(1);
  });

  test('exports an insight PDF rendered from the stored row', async ({ page }) => {
    await login(page);
    const insight = SEED.insight;

    await page.goto(`/insights/${insight.id}`, { waitUntil: 'domcontentloaded' });
    await expect(page.locator(`a[href="/insights/${insight.id}/pdf"][download]`)).toBeVisible();

    const response = await page.request.get(`/insights/${insight.id}/pdf`);
    expect(response.status()).toBe(200);
    expect(response.headers()['content-type']).toContain('application/pdf');
    expect(response.headers()['content-disposition']).toContain(`insight-${insight.id}.pdf`);
    const body = await response.body();
    expect(body.subarray(0, 4).toString('latin1')).toBe('%PDF');
    expect(body.length).toBeGreaterThan(1000);

    // The PDF is generated from the stored insight, so the row must exist.
    expect(
      await scalar('SELECT count(*)::int FROM insights WHERE id = $1::uuid', [insight.id])
    ).toBe(1);
  });

  test('saves preferences to user_preferences', async ({ page }) => {
    await login(page);

    await page.goto('/settings', { waitUntil: 'domcontentloaded' });
    await page.selectOption('#settings-theme', 'dark');
    await page.selectOption('#settings-table-layout', 'compact');
    await page.selectOption('#settings-region', 'EU');
    await page.selectOption('select[name="minimum_severity"]', 'high');
    await page.getByRole('button', { name: 'Save Preferences' }).click();
    await expect(page.locator('.apex-alert-success')).toContainText('Preferences saved.', {
      timeout: 15_000,
    });

    const row = (
      await q(
        `SELECT theme,
                preferences->'settings_page'->>'table_layout' AS table_layout,
                preferences->'settings_page'->>'default_region' AS default_region,
                preferences->'settings_page'->>'minimum_severity' AS minimum_severity
           FROM user_preferences WHERE user_id = 'admin'`
      )
    )[0];
    expect(row.theme).toBe('dark');
    expect(row.table_layout).toBe('compact');
    expect(row.default_region).toBe('EU');
    expect(row.minimum_severity).toBe('high');
  });

  test('watches and unwatches alerts for an entity', async ({ page }) => {
    await login(page);
    await q('DELETE FROM user_alert_subscriptions WHERE user_id = $1 AND entity_id = $2::uuid', [
      'admin',
      COMPANY_ID,
    ]);

    await page.goto(`/companies/${COMPANY_ID}`, { waitUntil: 'domcontentloaded' });
    await page.locator('[data-alert-subscription-severity]').selectOption('high');
    await page.locator('[data-alert-subscription-save]').click();
    await expect(page.locator('[data-alert-subscription-state]')).toHaveText(/Watching/, {
      timeout: 15_000,
    });

    await expectDb(
      async () =>
        scalar(
          `SELECT count(*)::int FROM user_alert_subscriptions
             WHERE user_id = 'admin' AND entity_id = $1::uuid AND enabled`,
          [COMPANY_ID]
        ),
      1
    );

    await page.locator('[data-alert-subscription-stop]').click();
    await expectDb(
      async () =>
        scalar(
          `SELECT count(*)::int FROM user_alert_subscriptions
             WHERE user_id = 'admin' AND entity_id = $1::uuid`,
          [COMPANY_ID]
        ),
      0
    );
  });

  test('traverses the graph from a stored edge', async ({ page }) => {
    await login(page);

    // The traversal must reflect the edge row, not a hardcoded fixture.
    expect(
      await scalar('SELECT count(*)::int FROM graph_edges WHERE id = $1::uuid', [
        JOURNEY.graphEdgeId,
      ])
    ).toBe(1);

    await page.goto('/graph', { waitUntil: 'domcontentloaded' });
    await page.fill('#graph-search-input', SEED.companies[0].name);
    await page.locator('#graph-load-neighborhood').waitFor({ state: 'visible', timeout: 15_000 });
    await page.locator('#graph-load-neighborhood').click();

    await expect(page.locator('#graph-search-status')).toContainText(/Loaded \d+ connected node/, {
      timeout: 15_000,
    });
    // The connected person from graph_edges must appear on the canvas.
    await expect(page.locator(`#graph-container g[data-nodeid="${PERSON_ID}"]`)).toHaveCount(1, {
      timeout: 15_000,
    });
  });

  test('regenerates a battlecard from stored data', async ({ page }) => {
    await login(page);

    await page.goto(`/battlecards/${JOURNEY.battlecardId}`, { waitUntil: 'domcontentloaded' });
    const regenerate = page.locator(
      `button[hx-post="/api/battlecards/${JOURNEY.battlecardId}/regenerate"]`
    );
    await expect(regenerate).toBeVisible();
    await regenerate.click();

    await expectDb(
      async () =>
        scalar('SELECT regenerated_at IS NOT NULL FROM battlecards WHERE id = $1::uuid', [
          JOURNEY.battlecardId,
        ]),
      true
    );
    const row = (
      await q(
        `SELECT positioning, updated_at, regenerated_at
           FROM battlecards WHERE id = $1::uuid`,
        [JOURNEY.battlecardId]
      )
    )[0];
    expect(row.positioning).not.toBeNull();
    expect(new Date(row.regenerated_at).getTime()).toBeGreaterThanOrEqual(
      new Date(row.updated_at).getTime() - 1000
    );
  });

  test('creates, edits, compares, exports and deletes a battlecard', async ({ page }) => {
    await login(page);
    page.on('dialog', (dialog) => dialog.accept());
    // Reverse of the seeded journey pair, so the two cards can be compared.
    const ours = SEED.companies[1];
    const rival = SEED.companies[0];
    await q(
      'DELETE FROM battlecards WHERE our_company_id = $1::uuid AND competitor_id = $2::uuid',
      [ours.id, rival.id]
    );

    // ── Create from the company page entry point ────────────────────────
    await page.goto(`/companies/${rival.id}`, { waitUntil: 'domcontentloaded' });
    await page.locator('[data-action="create-battlecard"]').click();
    await page.waitForURL(/\/battlecards\/new\?competitor_id=/);
    await expect(page.locator('select[name="competitor_id"]')).toHaveValue(rival.id);
    await page.fill('input[name="title"]', 'Journey battlecard');
    await page.selectOption('select[name="our_company_id"]', ours.id);
    await Promise.all([
      page.waitForURL(/\/battlecards\/[0-9a-f-]{36}\?notice=created$/),
      page.getByRole('button', { name: 'Create Battlecard' }).click(),
    ]);
    const id = page.url().match(/battlecards\/([0-9a-f-]{36})/)[1];
    await expect(page.locator('[data-battlecard-notice]')).toBeVisible();
    let row = (
      await q(
        'SELECT our_company_id, competitor_id, title, status FROM battlecards WHERE id = $1::uuid',
        [id]
      )
    )[0];
    expect(row).toEqual({
      our_company_id: ours.id,
      competitor_id: rival.id,
      title: 'Journey battlecard',
      status: 'draft',
    });

    // Creating the same pair again lands on the existing card.
    await page.goto(`/battlecards/new?competitor_id=${rival.id}&our_company_id=${ours.id}`, {
      waitUntil: 'domcontentloaded',
    });
    await page.fill('input[name="title"]', 'Duplicate attempt');
    await Promise.all([
      page.waitForURL(new RegExp(`/battlecards/${id}\\?notice=exists$`)),
      page.getByRole('button', { name: 'Create Battlecard' }).click(),
    ]);
    expect(
      await scalar(
        'SELECT count(*)::int FROM battlecards WHERE our_company_id = $1::uuid AND competitor_id = $2::uuid',
        [ours.id, rival.id]
      )
    ).toBe(1);

    // ── Edit title, status and a section ─────────────────────────────────
    await page.goto(`/battlecards/${id}/edit`, { waitUntil: 'domcontentloaded' });
    await page.fill('input[name="title"]', 'Journey battlecard (edited)');
    await page.selectOption('select[name="status"]', 'published');
    await page.locator('details:has(textarea[name="section_positioning"]) > summary').click();
    await page.fill('textarea[name="section_positioning"]', 'Journey positioning statement');
    await Promise.all([
      page.waitForURL(new RegExp(`/battlecards/${id}\\?notice=saved$`)),
      page.getByRole('button', { name: 'Save Changes' }).click(),
    ]);
    await expect(page.locator('[data-section="positioning"]')).toContainText(
      'Journey positioning statement'
    );
    row = (
      await q(
        'SELECT title, status, positioning, updated_by FROM battlecards WHERE id = $1::uuid',
        [id]
      )
    )[0];
    expect(row.title).toBe('Journey battlecard (edited)');
    expect(row.status).toBe('published');
    expect(row.positioning).toBe('Journey positioning statement');
    expect(row.updated_by).toBe('admin');

    // A concurrent change (e.g. a regeneration) makes the open editor stale.
    await page.goto(`/battlecards/${id}/edit`, { waitUntil: 'domcontentloaded' });
    await q("UPDATE battlecards SET updated_at = now() + interval '1 second' WHERE id = $1::uuid", [
      id,
    ]);
    await page.fill('input[name="title"]', 'Stale overwrite');
    await page.getByRole('button', { name: 'Save Changes' }).click();
    await expect(page.locator('[data-battlecard-error]')).toContainText(/changed after you opened/);
    expect(await scalar('SELECT title FROM battlecards WHERE id = $1::uuid', [id])).toBe(
      'Journey battlecard (edited)'
    );

    // ── Compare with the seeded card, then export both views ────────────
    await page.goto('/battlecards', { waitUntil: 'domcontentloaded' });
    await page.locator(`input[name="ids"][value="${id}"]`).check();
    await page.locator(`input[name="ids"][value="${JOURNEY.battlecardId}"]`).check();
    await Promise.all([
      page.waitForURL(/\/battlecards\/compare\?/),
      page.locator('[data-battlecard-compare]').click(),
    ]);
    const table = page.getByRole('table', { name: 'Battlecard comparison' });
    await expect(table).toContainText('Journey battlecard (edited)');
    await expect(table).toContainText('Northwind vs Cobalt');
    await expect(table).toContainText('Journey positioning statement');

    const exportHref = await page.locator('[data-battlecard-compare-export]').getAttribute('href');
    const comparison = await page.request.get(exportHref);
    expect(comparison.status()).toBe(200);
    expect(comparison.headers()['content-type']).toContain('text/markdown');
    expect(comparison.headers()['content-disposition']).toContain('battlecard-comparison.md');
    expect(await comparison.text()).toContain('Journey positioning statement');

    const single = await page.request.get(`/battlecards/${id}/export`);
    expect(single.status()).toBe(200);
    expect(single.headers()['content-disposition']).toContain(
      'battlecard-journey-battlecard-edited.md'
    );
    expect(await single.text()).toContain('Journey positioning statement');

    // ── Delete ───────────────────────────────────────────────────────────
    await page.goto(`/battlecards/${id}`, { waitUntil: 'domcontentloaded' });
    await Promise.all([
      page.waitForURL(/\/battlecards\?notice=deleted$/),
      page.locator('[data-battlecard-delete]').click(),
    ]);
    await expectDb(
      async () => scalar('SELECT count(*)::int FROM battlecards WHERE id = $1::uuid', [id]),
      0
    );
  });

  // The /admin page is read-only; the admin trigger surface is the security
  // page button (same `worker_trigger_queue` row) plus the admin JSON API.
  test('queues a worker job from the security trigger', async ({ page }) => {
    await login(page);

    await page.goto('/security', { waitUntil: 'domcontentloaded' });
    const trigger = page.locator('button[hx-post="/security/trigger-scan"]');
    await expect(trigger).toBeVisible();
    await trigger.click();
    await expect(page.locator('#security-trigger-status')).not.toBeEmpty({ timeout: 15_000 });

    // A queued, unclaimed dns_posture_scan job must exist — whether this click
    // inserted it or the queue deduplicated a still-pending row.
    await expect.poll(
      async () =>
        scalar(
          `SELECT count(*)::int FROM worker_trigger_queue
             WHERE job_kind = 'dns_posture_scan' AND completed_at IS NULL`
        ),
      { timeout: 15_000 }
    ).toBeGreaterThanOrEqual(1);
    const row = (
      await q(
        `SELECT claimed_at, completed_at FROM worker_trigger_queue
           WHERE job_kind = 'dns_posture_scan' ORDER BY requested_at DESC LIMIT 1`
      )
    )[0];
    expect(row.completed_at).toBeNull();
    expect(row.claimed_at).toBeNull();
  });

  test('queues a worker job through the admin trigger API', async ({ page, context }) => {
    await login(page);
    const cookies = await context.cookies();
    const csrf = cookies.find((cookie) => cookie.name === 'apex_csrf');
    expect(csrf, 'apex_csrf cookie must be issued to an authenticated session').toBeTruthy();

    const response = await page.request.post('/api/admin/trigger-scan', {
      headers: { 'x-csrf-token': csrf.value },
      data: { source_id: 'kev_catalog_fetch' },
    });
    expect(response.status()).toBe(200);
    const payload = await response.json();
    expect(payload.success).toBe(true);
    expect(payload.data.queued).toBe(true);

    // The id returned by the admin API must be a real, still-queued row (a new
    // insert or the deduplicated pending row) — never a fabricated success.
    const jobId = payload.data.job_id;
    expect(jobId).toBeTruthy();
    const rows = await q(
      `SELECT job_kind, completed_at, claimed_at FROM worker_trigger_queue WHERE id = $1::uuid`,
      [jobId]
    );
    expect(rows).toHaveLength(1);
    expect(rows[0].job_kind).toBe('kev_catalog_fetch');
    expect(rows[0].completed_at).toBeNull();
    expect(rows[0].claimed_at).toBeNull();
  });

  test('changes the password and can change it back', async ({ page, browser }) => {
    await login(page);
    const base =
      process.env.PLAYWRIGHT_BASE_URL || process.env.BASE_URL || 'http://127.0.0.1:9095';
    const user = process.env.ADMIN_USER || 'admin';
    const original = await scalar('SELECT password_hash FROM app_users WHERE id = $1', ['admin']);
    const NEW_PASSWORD = 'journey-password-123';

    const loginWith = async (target, password) => {
      await target.goto(`${base}/login`, { waitUntil: 'domcontentloaded' });
      await target.getByLabel(/^username$/i).fill(user);
      await target.getByLabel(/^password$/i).fill(password);
      await target.getByRole('button', { name: /sign in/i }).click();
    };

    try {
      // 1. Rotate through the UI. The acting session's cookie is reissued, so
      //    the page stays signed in after the version bump.
      await page.goto('/settings', { waitUntil: 'domcontentloaded' });
      await page.fill('#current-password', 'adminpassword');
      await page.fill('#new-password', NEW_PASSWORD);
      await page.fill('#confirm-password', NEW_PASSWORD);
      await page.getByRole('button', { name: 'Update Password' }).click();
      await page.waitForURL(/password_changed=1/, { timeout: 15_000 });

      const rotated = await scalar('SELECT password_hash FROM app_users WHERE id = $1', ['admin']);
      expect(rotated).not.toBe(original);
      expect(rotated.startsWith('$argon2id$')).toBe(true);

      // 2. The new password signs in (fresh context).
      const newContext = await browser.newContext();
      const newPage = await newContext.newPage();
      await loginWith(newPage, NEW_PASSWORD);
      await newPage.waitForURL((url) => !url.pathname.startsWith('/login'), {
        timeout: 15_000,
      });
      await newContext.close();

      // 3. The old password is rejected. Checked through the change form
      //    (invalid_current) rather than a wrong login, so this journey never
      //    feeds the login throttle and cannot lock the account for the rest
      //    of the suite.
      await page.goto('/settings', { waitUntil: 'domcontentloaded' });
      await page.fill('#current-password', 'adminpassword');
      await page.fill('#new-password', 'journey-password-999');
      await page.fill('#confirm-password', 'journey-password-999');
      await page.getByRole('button', { name: 'Update Password' }).click();
      await page.waitForURL(/password_error=invalid_current/, { timeout: 15_000 });
      await expect(page.getByText('The current password is incorrect.')).toBeVisible({
        timeout: 15_000,
      });

      // 4. Rotate back; the acting session is still signed in and the old
      //    credential works again. The deliberate failure above starts the
      //    progressive backoff, so retry the submit until the row changes;
      //    throttled attempts are refused *before* they count, so retrying
      //    cannot lock the endpoint.
      await expect
        .poll(
          async () => {
            if ((await page.locator('#current-password').count()) > 0) {
              const response = page
                .waitForResponse((r) => r.url().includes('/settings/password'), {
                  timeout: 5_000,
                })
                .catch(() => null);
              await page.fill('#current-password', NEW_PASSWORD);
              await page.fill('#new-password', 'adminpassword');
              await page.fill('#confirm-password', 'adminpassword');
              await page.getByRole('button', { name: 'Update Password' }).click();
              const resp = await response;
              if (resp) {
                await resp.finished();
              }
            }
            return scalar('SELECT password_hash FROM app_users WHERE id = $1', ['admin']);
          },
          { timeout: 90_000 }
        )
        .not.toBe(rotated);

      const restoredContext = await browser.newContext();
      const restoredPage = await restoredContext.newPage();
      await loginWith(restoredPage, 'adminpassword');
      await restoredPage.waitForURL((url) => !url.pathname.startsWith('/login'), {
        timeout: 15_000,
      });
      await restoredContext.close();
    } finally {
      // Always restore the canonical fixture credential, even on failure.
      await q(
        'UPDATE app_users SET password_hash = $1, session_version = session_version + 1 WHERE id = $2',
        [original, user]
      );
    }
  });
});
