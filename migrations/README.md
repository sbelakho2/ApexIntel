# ApexIntel Migration Directory — `migrations/`

This directory contains the **primary, canonical** migration lineage for ApexIntel.

## Relationship with `crates/store/migrations/`

There is a **secondary** migration lineage at `crates/store/migrations/` which was
developed independently and defines the same tables with different column names/types.

**The `migrations/` directory is authoritative.** The `crates/store/migrations/`
lineage exists for backward compatibility with older deployments.

## Key Differences

| Feature | `migrations/` (primary) | `crates/store/migrations/` (secondary) |
|---------|------------------------|----------------------------------------|
| Naming | Date-based (YYYYMMDD_desc) | Sequential (NNNN_desc) |
| `recipes` PK | `id TEXT` | `code TEXT` |
| `warnings` entity ref | `entity_id UUID` (scalar) | `entity_ids UUID[]` (array) |
| `insights` entity ref | `entity_id UUID` (scalar) | `entity_ids UUID[]` (array) |
| `feature_rows` | Typed columns per feature | Single `data JSONB` catch-all |

## Unification

Migration `20260514_unify_duplicate_schemas.sql` bridges both lineages by:
1. Adding store-crate columns to core-schema tables (with triggers to sync them)
2. Adding CHECK constraints, indexes, and RLS policies
3. Creating triggers to keep `entity_id` ↔ `entity_ids` and `recipe_id` ↔ `recipe_code` in sync

## Always Run in Order

Apply migrations in filename order (date ascending).
