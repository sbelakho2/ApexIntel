# ApexIntel Migration Directory — `migrations/`

This directory contains the **canonical** migration lineage for ApexIntel.

## Single lineage

This is the **only** migration lineage. `apex-store` embeds it with
`sqlx::migrate!("../../migrations")` and production checksum-verifies it at boot.
A second, independently developed lineage used to live at
`crates/store/migrations/` (different column names/types for the same tables);
it was deleted and `scripts/ci/check_single_migrations_folder.sh` now fails the
build if that directory or a second `sqlx::migrate!` source path reappears.

Historical reference — the old secondary lineage differed from this one in:

| Feature | `migrations/` (authoritative) | `crates/store/migrations/` (deleted) |
|---------|------------------------------|--------------------------------------|
| Naming | Sequential (NNN_desc) | Sequential (NNNN_desc, legacy) |
| `recipes` PK | `id TEXT` | `code TEXT` |
| `warnings` entity ref | `entity_id UUID` (scalar) | `entity_ids UUID[]` (array) |
| `insights` entity ref | `entity_id UUID` (scalar) | `entity_ids UUID[]` (array) |
| `feature_rows` | Typed columns per feature | Single `data JSONB` catch-all |

## Skipped numbers

The numbering is intentionally not contiguous: `066`, `067` and `095` never
existed in this tree (verified against git history), and no migration may be
renumbered to fill them — `000`–`078` are production-applied and byte-frozen in
`scripts/ci/frozen_migrations.txt`. sqlx orders by version and does not require
contiguous numbers; `scripts/ci/check_migration_headers.sh` treats every
migration above `FROZEN_MAX` (78) as pending and requires the next new file to
state its own number.

## Historical unification

Migration `021_unify_duplicate_schemas.sql` bridged the two old lineages by:
1. Adding store-crate columns to core-schema tables (with triggers to sync them)
2. Adding CHECK constraints, indexes, and RLS policies
3. Creating triggers to keep `entity_id` ↔ `entity_ids` and `recipe_id` ↔ `recipe_code` in sync

## Always Run in Order

Apply migrations in filename order (numeric ascending).
