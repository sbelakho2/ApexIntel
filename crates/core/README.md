# apex-core

Shared domain types, validation helpers, and error definitions used across the entire ApexIntel workspace.

## Responsibilities

- **Domain entities** (`entities.rs`): All canonical structs and enums for the knowledge graph — `Company`, `Site`, `Person`, `Observation`, `GraphEdge`, `Certification`, `Capability`, `FeatureRow`, etc.
- **Error types** (`errors.rs`): `ApexError` enum and `Result<T>` alias.  Every crate returns this error type so callers and the API layer can map errors consistently.
- **Validation helpers** (`validation.rs`): Pure utility functions shared across all crates:
  - `validate_nonempty_id`, `is_safe_id` — ID format guards
  - `safe_div`, `round_to_dp` — numeric safety
  - `trim_user_string`, `normalize_unicode_whitespace` — input sanitisation
  - `validate_uuid`, `validate_country_code`, `validate_region_code` — field-level validators
  - `timestamp_to_days_checked` — overflow-safe timestamp conversion
- **Provenance** (`provenance.rs`): `Provenance` and `ContentSnapshot` for source tracking and change detection.
- **Outcomes** (`outcomes.rs`): `OutcomeEvent` and `OutcomeRecord` for audit-trail events.
- **Schemas** (`schemas.rs`): Shared JSON schema helpers.
- **Config** (`config.rs`): `AppConfig` loaded from environment variables.

## Key types

| Type | Purpose |
|------|---------|
| `ApexError` | Unified error enum for the workspace |
| `Company` | Supply-chain company node |
| `Person` | Person-of-Interest (POI) |
| `Observation` | Time-stamped raw signal from the crawl stage |
| `GraphEdge` | Typed directed edge in the supply-chain graph |
| `FeatureRow` | Time-bucketed ML feature vector per entity |

## Design principles

- Zero async code in this crate — pure data and validation.
- All structs derive `Debug`, `Clone`, `Serialize`, `Deserialize`.
- No database dependencies.
