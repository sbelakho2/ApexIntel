// Deterministic server-UI fixtures for the Playwright contract specs, the
// task-budget flows in scripts/ci/e2e_server_ui.mjs, and the seeded visual
// baselines.
//
// Inserts (idempotently) a small, fixed corpus: companies, warnings with
// source evidence and semantic-merge bookkeeping, people + a buying centre, an
// insight with traceable claims, a company-linked pipeline opportunity, and an
// open investigation workspace. All ids and timestamps are fixed so
// screenshots and assertions are stable across runs.
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
  // Repeated observations of warning #1 are tracked by the semantic-merge
  // engine on `triage_queue`: 3 occurrences, merged source domains, and an
  // escalated effective severity (high -> critical).
  triageMerge: {
    id: '7e570000-0000-4000-8000-000000000001',
    itemType: 'warning',
    sourceId: '0ddba110-0000-4000-8000-000000000001',
    title: 'Northwind Power expands cell manufacturing capacity',
    staticSeverity: 'critical',
    occurrenceCount: 3,
    firstSeenAt: '2026-01-15 08:30:00+00',
    lastSeenAt: '2026-01-20 11:00:00+00',
    mergedSourceUrls: [
      'https://example.test/reports/northwind-capacity-expansion',
      'https://example.test/permits/northwind-austin-2026',
      'https://news.example.test/northwind-austin-hiring',
    ],
  },
  persons: [
    {
      id: 'beef0000-0000-4000-8000-000000000001',
      name: 'Dana Whitfield',
      currentRole: 'Chief Executive Officer',
      roleFamily: 'Executive',
      region: 'US',
      bio: 'Leads the Northwind Power Systems grid-scale expansion programme.',
      influenceScore: 0.92,
      companyId: 'c0ffee00-0000-4000-8000-000000000001',
    },
    {
      id: 'beef0000-0000-4000-8000-000000000002',
      name: 'Marco Ilves',
      currentRole: 'VP Procurement',
      roleFamily: 'Procurement',
      region: 'EU',
      bio: 'Owns cell and logistics sourcing for Northwind Power Systems.',
      influenceScore: 0.71,
      companyId: 'c0ffee00-0000-4000-8000-000000000001',
    },
  ],
  buyingCenter: {
    id: 'bc000000-0000-4000-8000-000000000001',
    name: 'Northwind Power Systems Buying Center',
    companyId: 'c0ffee00-0000-4000-8000-000000000001',
    status: 'engaged',
    members: [
      {
        personId: 'beef0000-0000-4000-8000-000000000001',
        role: 'decision_maker',
        influenceScore: 0.92,
        budgetAuthority: true,
      },
      {
        personId: 'beef0000-0000-4000-8000-000000000002',
        role: 'economic_buyer',
        influenceScore: 0.71,
        budgetAuthority: false,
      },
    ],
  },
  insight: {
    id: '1d5eaf00-0000-4000-8000-000000000001',
    evidenceRefId: 'e71de000-0000-4000-8000-000000000001',
    evidenceUrl: 'https://example.test/analysis/northwind-evidence',
    companyId: 'c0ffee00-0000-4000-8000-000000000001',
    insightType: 'capacity_expansion',
    title: 'Northwind capacity expansion shifts grid storage supply',
    summary: 'Cell line expansion adds 4 GWh of supply within three quarters.',
    narrative:
      'Northwind Power Systems filed permits and posted supplier roles consistent with a 4 GWh cell line expansion. The added capacity is expected to shift grid-scale storage supply within three quarters.',
    confidence: 0.83,
    region: 'US',
    tags: ['capacity', 'supply'],
    claim: 'Northwind added 4 GWh of cell capacity in Austin.',
    claimKind: 'observed',
    createdAt: '2026-01-17 10:00:00+00',
  },
  pipeline: {
    id: '9e7e0000-0000-4000-8000-000000000001',
    companyId: 'c0ffee00-0000-4000-8000-000000000001',
    title: 'Northwind grid storage capacity opportunity',
    stage: 'discovery',
    valueEstimate: 250000,
    probability: 0.35,
  },
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

      // Remove previous rows so repeated runs (and repeats after the ack /
      // bookmark / investigate task flows) always start from the same state.
      await client.query('DELETE FROM warnings WHERE id = ANY($1::uuid[])', [
        SEED.warnings.map((w) => w.id),
      ]);
      await client.query('DELETE FROM insights WHERE id = $1::uuid', [SEED.insight.id]);
      await client.query('DELETE FROM persons WHERE id = ANY($1::uuid[])', [
        SEED.persons.map((p) => p.id),
      ]);
      await client.query('DELETE FROM buying_centers WHERE id = $1::uuid', [
        SEED.buyingCenter.id,
      ]);
      await client.query('DELETE FROM pipeline_opportunities WHERE id = $1::uuid', [
        SEED.pipeline.id,
      ]);
      await client.query('DELETE FROM triage_queue WHERE id = $1::uuid', [
        SEED.triageMerge.id,
      ]);
      await client.query('DELETE FROM investigation_workspaces WHERE id = ANY($1::uuid[])', [
        [SEED.workspace.id],
      ]);
      // Workspaces created by the "Start investigation" task flow are
      // regenerated on every run; drop the deterministic ones first.
      await client.query(
        "DELETE FROM investigation_workspaces WHERE name = $1",
        [`Investigate: ${SEED.warnings[0].title}`]
      );

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

      const merge = SEED.triageMerge;
      await client.query(
        `INSERT INTO triage_queue
           (id, item_type, source_id, title, description, entity_id, entity_name,
            static_severity, composite_score, status, occurrence_count,
            last_seen_at, merged_source_urls, created_at, updated_at)
         VALUES ($1, $2, $3::uuid, $4, $5, $6::uuid, $7, $8, 0.75, 'pending', $9,
                 $10::timestamptz, $11::text[], $12::timestamptz, $12::timestamptz)
         ON CONFLICT (id) DO UPDATE SET
           title = EXCLUDED.title,
           static_severity = EXCLUDED.static_severity,
           occurrence_count = EXCLUDED.occurrence_count,
           last_seen_at = EXCLUDED.last_seen_at,
           merged_source_urls = EXCLUDED.merged_source_urls,
           updated_at = EXCLUDED.updated_at`,
        [
          merge.id,
          merge.itemType,
          merge.sourceId,
          merge.title,
          SEED.warnings[0].description,
          SEED.warnings[0].companyId,
          SEED.companies[0].name,
          merge.staticSeverity,
          merge.occurrenceCount,
          merge.lastSeenAt,
          merge.mergedSourceUrls,
          merge.firstSeenAt,
        ]
      );

      for (const person of SEED.persons) {
        await client.query(
          `INSERT INTO persons
             (id, name, primary_org_id, "current_role", role_family, region,
              public_bio, influence_score, created_at, updated_at)
           VALUES ($1, $2, $3::uuid, $4, $5, $6, $7, $8,
                   TIMESTAMPTZ '2026-01-10 09:00:00+00', TIMESTAMPTZ '2026-01-16 09:00:00+00')
           ON CONFLICT (id) DO UPDATE SET
             name = EXCLUDED.name,
             primary_org_id = EXCLUDED.primary_org_id,
             "current_role" = EXCLUDED."current_role",
             role_family = EXCLUDED.role_family,
             region = EXCLUDED.region,
             public_bio = EXCLUDED.public_bio,
             influence_score = EXCLUDED.influence_score,
             updated_at = EXCLUDED.updated_at`,
          [
            person.id,
            person.name,
            person.companyId,
            person.currentRole,
            person.roleFamily,
            person.region,
            person.bio,
            person.influenceScore,
          ]
        );
      }

      const center = SEED.buyingCenter;
      await client.query(
        `INSERT INTO buying_centers (id, opportunity_id, company_id, name, deal_value, status)
         VALUES ($1::uuid, NULL, $2::uuid, $3, NULL, $4)
         ON CONFLICT (id) DO UPDATE SET
           name = EXCLUDED.name,
           status = EXCLUDED.status`,
        [center.id, center.companyId, center.name, center.status]
      );
      for (const member of center.members) {
        await client.query(
          `INSERT INTO buying_center_members
             (buying_center_id, person_id, role, influence_score, budget_authority, need_signal)
           VALUES ($1::uuid, $2::uuid, $3, $4, $5, 0.5)
           ON CONFLICT (buying_center_id, person_id) DO UPDATE SET
             role = EXCLUDED.role,
             influence_score = EXCLUDED.influence_score,
             budget_authority = EXCLUDED.budget_authority`,
          [center.id, member.personId, member.role, member.influenceScore, member.budgetAuthority]
        );
      }

      const insight = SEED.insight;
      await client.query(
        `INSERT INTO insights
           (id, insight_type, title, summary, narrative, region, confidence,
            entity_id, entity_ids, evidence_urls, tags, metadata, created_at, updated_at)
         VALUES ($1::uuid, $2, $3, $4, $5, $6, $7, $8::uuid, ARRAY[$8]::uuid[],
                 $9::text[], $10::text[],
                 jsonb_build_object('evidence_refs', jsonb_build_array(
                   jsonb_build_object('id', $11::text, 'url', $12::text))),
                 $13::timestamptz, $13::timestamptz)
         ON CONFLICT (id) DO UPDATE SET
           title = EXCLUDED.title,
           summary = EXCLUDED.summary,
           narrative = EXCLUDED.narrative,
           confidence = EXCLUDED.confidence,
           evidence_urls = EXCLUDED.evidence_urls,
           metadata = EXCLUDED.metadata,
           updated_at = EXCLUDED.updated_at`,
        [
          insight.id,
          insight.insightType,
          insight.title,
          insight.summary,
          insight.narrative,
          insight.region,
          insight.confidence,
          insight.companyId,
          [insight.evidenceUrl],
          insight.tags,
          insight.evidenceRefId,
          insight.evidenceUrl,
          insight.createdAt,
        ]
      );
      await client.query('DELETE FROM insight_claims WHERE insight_id = $1::uuid', [
        insight.id,
      ]);
      await client.query(
        `INSERT INTO insight_claims
           (insight_id, claim, evidence_ids, confidence, claim_kind, claim_hash)
         VALUES ($1::uuid, $2, ARRAY[$3]::uuid[], $4, $5, 'seed-claim-1')`,
        [
          insight.id,
          insight.claim,
          insight.evidenceRefId,
          insight.confidence,
          insight.claimKind,
        ]
      );

      const pipeline = SEED.pipeline;
      await client.query(
        `INSERT INTO pipeline_opportunities
           (id, title, stage, value_estimate, probability, company_id, created_at, updated_at)
         VALUES ($1::uuid, $2, $3, $4, $5, $6::uuid,
                 TIMESTAMPTZ '2026-01-16 09:00:00+00', TIMESTAMPTZ '2026-01-16 09:00:00+00')
         ON CONFLICT (id) DO UPDATE SET
           title = EXCLUDED.title,
           stage = EXCLUDED.stage,
           value_estimate = EXCLUDED.value_estimate,
           probability = EXCLUDED.probability,
           company_id = EXCLUDED.company_id,
           updated_at = EXCLUDED.updated_at`,
        [
          pipeline.id,
          pipeline.title,
          pipeline.stage,
          pipeline.valueEstimate,
          pipeline.probability,
          pipeline.companyId,
        ]
      );

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
