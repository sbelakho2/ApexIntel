// Deterministic server-UI fixtures for the Playwright contract specs and the
// seeded visual baselines.
//
// Inserts (idempotently) a small, fixed corpus: companies, warnings with
// source evidence, and an open investigation workspace. All ids and timestamps
// are fixed so screenshots and assertions are stable across runs.
//
// Usage (CLI): DATABASE_URL=postgres://... node scripts/ci/seed_server_ui_fixtures.mjs
// Usage (specs): const { seedDatabase, login, SEED } = require('../../e2e/helpers/server-ui-fixtures.cjs');

'use strict';

const { Pool } = require('pg');

const SEED = {
  companies: [
    {
      id: 'c0ffee00-0000-4000-8000-000000000001',
      name: 'Northwind Power Systems',
      legalName: 'Northwind Power Systems Ltd',
      domain: 'northwind-power.test',
      region: 'US',
      companyType: 'manufacturer',
      industryTags: ['battery', 'grid-scale', 'energy'],
      riskScore: 0.64,
      threatScore: 0.72,
      overlapScore: 0.35,
    },
    {
      id: 'c0ffee00-0000-4000-8000-000000000002',
      name: 'Cobalt Grid Logistics',
      legalName: 'Cobalt Grid Logistics GmbH',
      domain: 'cobalt-grid.test',
      region: 'EU',
      companyType: 'logistics',
      industryTags: ['logistics', 'cobalt'],
      riskScore: 0.41,
      threatScore: 0.28,
      overlapScore: 0.12,
    },
  ],
  warnings: [
    {
      id: '0ddba110-0000-4000-8000-000000000001',
      companyId: 'c0ffee00-0000-4000-8000-000000000001',
      warningType: 'capacity_alert',
      severity: 'high',
      title: 'Northwind Power expands cell manufacturing capacity',
      description:
        'Permit filings and supplier job postings indicate a 4 GWh cell line expansion at the Northwind Power Systems Austin campus.',
      region: 'US',
      confidence: 0.83,
      sourceUrls: [
        'https://example.test/reports/northwind-capacity-expansion',
        'https://example.test/permits/northwind-austin-2026',
      ],
      acknowledged: false,
      tsUtc: '2026-01-15 08:30:00+00',
    },
    {
      id: '0ddba110-0000-4000-8000-000000000002',
      companyId: 'c0ffee00-0000-4000-8000-000000000002',
      warningType: 'supplier_risk',
      severity: 'medium',
      title: 'Cobalt Grid Logistics adds single-source port dependency',
      description:
        'Route analysis shows Cobalt Grid Logistics concentrating EU cobalt flows through a single Baltic port terminal.',
      region: 'EU',
      confidence: 0.61,
      sourceUrls: ['https://example.test/analysis/cobalt-baltic-route'],
      acknowledged: false,
      tsUtc: '2026-01-14 16:45:00+00',
    },
  ],
  workspace: {
    id: 'aaaa1111-0000-4000-8000-000000000001',
    name: 'Northwind capacity investigation',
    description:
      'Track the Northwind Power Systems capacity expansion and its effect on grid-scale procurement.',
    workspaceType: 'structured',
    ownerId: 'admin',
    status: 'active',
    visibility: 'team',
    tags: ['supply-chain', 'capacity'],
    createdAt: '2026-01-16 09:00:00+00',
  },
};

function databaseUrl() {
  const url = process.env.DATABASE_URL || process.env.TEST_DATABASE_URL;
  if (!url) {
    throw new Error('DATABASE_URL (or TEST_DATABASE_URL) must be set to seed server-UI fixtures');
  }
  return url;
}

async function seedDatabase() {
  const pool = new Pool({ connectionString: databaseUrl() });
  try {
    const client = await pool.connect();
    try {
      await client.query('BEGIN');

      // Remove previous rows so repeated runs (and repeats after the ack test)
      // always start from the same canonical state.
      await client.query('DELETE FROM warnings WHERE id = ANY($1::uuid[])', [
        SEED.warnings.map((w) => w.id),
      ]);
      await client.query('DELETE FROM investigation_workspaces WHERE id = ANY($1::uuid[])', [
        [SEED.workspace.id],
      ]);

      for (const company of SEED.companies) {
        await client.query(
          `INSERT INTO companies
             (id, name, legal_name, domain, region, company_type, industry_tags,
              risk_score, threat_score, overlap_score, is_competitor, metadata,
              created_at, updated_at)
           VALUES ($1, $2, $3, $4, $5, $6, $7::text[], $8, $9, $10, FALSE, '{}'::jsonb,
                   TIMESTAMPTZ '2026-01-10 09:00:00+00', TIMESTAMPTZ '2026-01-16 09:00:00+00')
           ON CONFLICT (id) DO UPDATE SET
             name = EXCLUDED.name,
             legal_name = EXCLUDED.legal_name,
             domain = EXCLUDED.domain,
             region = EXCLUDED.region,
             company_type = EXCLUDED.company_type,
             industry_tags = EXCLUDED.industry_tags,
             risk_score = EXCLUDED.risk_score,
             threat_score = EXCLUDED.threat_score,
             overlap_score = EXCLUDED.overlap_score,
             updated_at = EXCLUDED.updated_at`,
          [
            company.id,
            company.name,
            company.legalName,
            company.domain,
            company.region,
            company.companyType,
            company.industryTags,
            company.riskScore,
            company.threatScore,
            company.overlapScore,
          ]
        );
      }

      for (const warning of SEED.warnings) {
        await client.query(
          `INSERT INTO warnings
             (id, warning_type, severity, title, description, entity_id, entity_ids,
              region, confidence, source_urls, acknowledged, ts_utc, metadata,
              created_at, updated_at)
           VALUES ($1, $2, $3, $4, $5, $6, ARRAY[$6]::uuid[], $7, $8, $9::text[],
                   FALSE, $10::timestamptz, '{}'::jsonb, $10::timestamptz, $10::timestamptz)
           ON CONFLICT (id) DO UPDATE SET
             title = EXCLUDED.title,
             description = EXCLUDED.description,
             severity = EXCLUDED.severity,
             region = EXCLUDED.region,
             confidence = EXCLUDED.confidence,
             source_urls = EXCLUDED.source_urls,
             acknowledged = FALSE,
             acknowledged_by = NULL,
             acknowledged_at = NULL,
             acknowledged_note = NULL,
             updated_at = EXCLUDED.updated_at`,
          [
            warning.id,
            warning.warningType,
            warning.severity,
            warning.title,
            warning.description,
            warning.companyId,
            warning.region,
            warning.confidence,
            warning.sourceUrls,
            warning.tsUtc,
          ]
        );
      }

      await client.query(
        `INSERT INTO investigation_workspaces
           (id, name, description, workspace_type, owner_id, status, visibility,
            tags, entity_focus, metadata, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8::text[], $9::jsonb, '{}'::jsonb,
                 $10::timestamptz, $10::timestamptz)
         ON CONFLICT (id) DO UPDATE SET
           name = EXCLUDED.name,
           description = EXCLUDED.description,
           status = EXCLUDED.status,
           visibility = EXCLUDED.visibility,
           tags = EXCLUDED.tags,
           entity_focus = EXCLUDED.entity_focus,
           updated_at = EXCLUDED.updated_at`,
        [
          SEED.workspace.id,
          SEED.workspace.name,
          SEED.workspace.description,
          SEED.workspace.workspaceType,
          SEED.workspace.ownerId,
          SEED.workspace.status,
          SEED.workspace.visibility,
          SEED.workspace.tags,
          JSON.stringify([SEED.companies[0].id]),
          SEED.workspace.createdAt,
        ]
      );

      await client.query('COMMIT');
    } catch (error) {
      await client.query('ROLLBACK');
      throw error;
    } finally {
      client.release();
    }
  } finally {
    await pool.end();
  }
  return SEED;
}

/**
 * Log into the server UI with the admin credentials and land on the dashboard.
 * @param {import('@playwright/test').Page} page
 */
async function login(page) {
  const baseURL = process.env.PLAYWRIGHT_BASE_URL || process.env.BASE_URL || 'http://127.0.0.1:9095';
  const user = process.env.ADMIN_USER || 'admin';
  const pass = process.env.ADMIN_PASS || 'adminpassword';

  await page.goto(`${baseURL}/login`, { waitUntil: 'domcontentloaded' });
  await page.getByRole('textbox', { name: /operator id/i }).fill(user);
  await page.getByRole('textbox', { name: /access key/i }).fill(pass);
  await page.getByRole('button', { name: /access platform/i }).click();
  await page.waitForURL((url) => !url.pathname.startsWith('/login'), { timeout: 15_000 });
}

module.exports = { SEED, seedDatabase, login, databaseUrl };
