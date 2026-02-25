# apex-crawl

Polite, resilient web crawling — rate limiting, proxy rotation, robots.txt enforcement, change detection, header management, and live metrics.

## Modules

| Module | Responsibility |
|--------|---------------|
| `rate_limit.rs` | Per-engine adaptive backoff with jitter; `RateLimitManager` tracks failure counts and cooldown windows |
| `governor_limiter.rs` | Token-bucket rate limiting via the `governor` crate (fixed request-per-second ceiling) |
| `headers.rs` | HTTP header canonicalisation, user-agent rotation, referrer policy helpers |
| `proxy.rs` | Proxy pool selection, rotation policy, health tracking |
| `robots.rs` | Robots.txt fetch, parse, and per-engine `is_allowed()` enforcement |
| `change_detection.rs` | SHA-256 content fingerprinting with LRU-bounded in-memory cache; strips volatile content (timestamps, copyright lines) before hashing |
| `metrics.rs` | Counters and histograms for crawl success rates, bytes fetched, latency percentiles |

## Rate limiting strategy

`RateLimitManager` maintains per-engine state:

- **Base delay**: 3 s between requests.
- **Backoff**: each failure multiplies delay (exponential), capped at 60 s per request and 30 min total cooldown.
- **Jitter**: ±30 % random jitter applied to every calculated delay to avoid thundering-herd.
- **Success streak reset**: 2 consecutive successes halve the current backoff.
- **Stale eviction**: engines not seen for > 1 h are evicted from the map to bound memory (cap: 10 000 engines).

`EngineStateSnapshot` provides a serialisable view for persistence across restarts.

## Change detection

`ChangeDetector` fingerprints crawled content:

1. Strip volatile sub-strings (timestamps, `© YYYY`, HH:MM:SS patterns) via a compile-once `LazyLock<Regex>`.
2. Compute `SHA-256(source_url + stripped_content)` to avoid cross-source collisions.
3. Maintain an LRU map bounded by `max_capacity` (default 100 000 URLs).

For persistent deduplication across restarts use the `store` crate's PostgreSQL backend.

## Robots.txt

`robots.rs` fetches and caches `robots.txt` per host.  `is_allowed(url, user_agent)` returns `false` for disallowed paths and for hosts that cannot be reached (fail-closed).

## Key invariants

- No crawl request is issued without a `robots.txt` check.
- All delays include jitter — deterministic polling is forbidden.
- Proxies are health-checked before selection; unhealthy proxies are skipped.
- Metrics are emitted for every request outcome (success, 4xx, 5xx, timeout).

## Environment variables

```
PROXY_LIST=http://p1:port,http://p2:port   # comma-separated
CRAWL_USER_AGENT=ApexIntel/1.0
CRAWL_MAX_RPS=2                            # per engine
```
