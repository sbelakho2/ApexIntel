# apex-store

Persistence layer — PostgreSQL entity storage, S3 object storage, Tantivy full-text search, and in-memory feature store.

## Modules

| Module | Responsibility |
|--------|---------------|
| `postgres.rs` | All SQL read/write for every domain entity (companies, sites, people, POI, observations, graph edges, warnings, insights, certifications, …) |
| `s3.rs` | Object upload/download, presigned URL generation, content-type inference |
| `tantivy_index.rs` | Full-text index build and search over company & insight documents |
| `feature_store.rs` | In-memory time-series feature aggregation with daily/weekly/monthly bucketing |

## Key design decisions

### ILIKE safety
User-supplied search strings are passed through `ilike_pattern()`, which escapes `%`, `_`, and `\` before embedding in SQL `ILIKE` clauses.  This prevents unintentional wildcard expansion.

### List limit clamp
`MAX_LIST_LIMIT = 500`.  All list queries call `clamp_limit(limit)` so a client can never force an unbounded table scan.

### URL normalisation at the DB boundary
All URL columns are normalised by `normalize_url()` before insert/update (trims scheme, trailing slash, etc.).  Invalid URLs are silently dropped from batch inputs through `normalize_url_vec()`.

### Tag length validation
Tags are capped at 64 Unicode code points.  `validate_tags()` is called before every insert.

## Feature store

`BucketSize` (`Daily = 1`, `Weekly = 7`, `Monthly = 30`) controls aggregation granularity.  `time_bucket(ts, bucket)` uses `div_euclid` so negative timestamps (pre-1970) bucket correctly.

`FeatureMatrix::merge()` combines multiple entity `FeatureRow` slices and normalises columns to a common universe via zero-filling.

## Migrations

SQL migrations live in `migrations/` at the workspace root.  Run with `sqlx migrate run` or prisma tooling.

## Connection pool

Callers construct a `PgPool` via `PgPoolOptions::new()` and pass it into query helpers.  Pool sizing defaults (`max_connections = 20`, acquire timeout 30 s) are configured via environment variable.

## Environment variables

```
DATABASE_URL=postgres://...
S3_BUCKET=...
S3_REGION=us-east-1
AWS_ACCESS_KEY_ID=...
AWS_SECRET_ACCESS_KEY=...
```
