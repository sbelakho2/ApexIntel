# ApexIntel Store Migration Directory — `crates/store/migrations/`

This directory contains the **secondary** migration lineage for ApexIntel.

## Relationship with `migrations/`

The **primary** migration lineage lives at `migrations/` (top-level project directory).
This directory exists for backward compatibility with older deployments that use the
sequential-numbering migration scheme.

**Do NOT add new migrations here.** All new migrations should go in the top-level
`migrations/` directory with date-based naming.

## Schema Reconciliation

The unification migration `migrations/20260514_unify_duplicate_schemas.sql` bridges
the schema differences between this lineage and the primary lineage. Key differences:

- `recipes(code)` → `recipes(id)` (alias via trigger)
- `warnings.entity_ids UUID[]` → `warnings.entity_id UUID` (synced via trigger)
- `insights.entity_ids UUID[]` → `insights.entity_id UUID` (synced via trigger)

## Migration Ordering

These migrations use sequential numbering (`0001_`, `0002_`, ...). They should be
applied AFTER the primary `migrations/` lineage, or skip them entirely in favor of
the primary lineage (recommended for new deployments).
