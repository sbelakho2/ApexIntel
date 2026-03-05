# ApexIntel Operational Runbook

> Comprehensive operational guide for ApexIntel competitive intelligence platform.
> Last updated: 2026-03-01

---

## Table of Contents

1. [First-Time Setup](#first-time-setup)
2. [Architecture Overview](#architecture-overview)
3. [Daily Operations](#daily-operations)
4. [Common Failure Modes](#common-failure-modes)
5. [Recovery Procedures](#recovery-procedures)
6. [Performance Tuning](#performance-tuning)
7. [Scaling Guidelines](#scaling-guidelines)
8. [Security Checklist](#security-checklist)
9. [Incident Playbook](#incident-playbook)
10. [Monitoring & Alerting](#monitoring--alerting)

---

## First-Time Setup

### Prerequisites

- Linux VPS (Ubuntu 22.04+ or Debian 12+), minimum 8 cores / 32GB RAM / 500GB NVMe
- Domain with DNS pointing to server (e.g. `starzerp.fi` → `77.42.65.89`)
- SSL certificate (Let's Encrypt recommended)
- Rust 1.77+ toolchain
- Node.js 20+ with npm
- Docker & Docker Compose v2

### Step-by-Step (estimated: 15 minutes)

```bash
# 1. Clone repository
git clone git@github.com:org/ApexIntel.git /opt/apexintel
cd /opt/apexintel

# 2. Configure environment
cp .env.example .env
# Edit .env — fill in:
#   DATABASE_URL, REDIS_URL, NATS_URL, MINIO_*
#   JWT_SECRET (generate: openssl rand -hex 32)
#   SCRAPER_USER_AGENT, SCRAPER_PROXY_URL (optional)
#   LLM_SERVER_URL (default: http://localhost:8081)
nano .env

# 3. Start infrastructure services
docker compose up -d
# Wait for PostgreSQL health check (~10 seconds)
sleep 10

# 4. Run database migrations (in order)
psql "$DATABASE_URL" < migrations/00000000_core_schema.sql
psql "$DATABASE_URL" < migrations/20260228_dossier_and_role_history.sql
psql "$DATABASE_URL" < migrations/20260228_weekly_memos_and_competitor_changes.sql
psql "$DATABASE_URL" < migrations/20260301_data_retention.sql
psql "$DATABASE_URL" < migrations/20260301_materialized_views.sql
psql "$DATABASE_URL" < migrations/20260301_observation_dedup.sql

# 5. Seed initial data
psql "$DATABASE_URL" < scripts/seed_data.sql
psql "$DATABASE_URL" < scripts/seed_competitors_and_pois.sql

# 6. Build Rust binaries
cargo build --release
# Produces: target/release/apex-api, target/release/apex-worker

# 7. Build frontend
cd frontend
npm install
npm run build
cd ..

# 8. Start services (or use systemd — see below)
./target/release/apex-api &
./target/release/apex-worker &
cd frontend && npm start &

# 9. Verify
curl -s http://localhost:8080/api/health | jq .
# Expected: {"status":"ok","version":"...","uptime_secs":...}

curl -s http://localhost:8080/api/health/deep | jq .
# Expected: all components "ok"
```

### Systemd Installation

```bash
# Copy service files
sudo cp config/systemd/apexintel-api.service /etc/systemd/system/
sudo cp config/systemd/apexintel-worker.service /etc/systemd/system/
sudo cp config/systemd/apexintel-frontend.service /etc/systemd/system/
sudo cp config/systemd/apexintel-llm.service /etc/systemd/system/

# Reload and enable
sudo systemctl daemon-reload
sudo systemctl enable --now apexintel-api apexintel-worker apexintel-frontend

# Verify
sudo systemctl status apexintel-api
sudo systemctl status apexintel-worker
```

### Nginx Setup

```bash
sudo cp config/nginx/apexintel.conf /etc/nginx/sites-available/apexintel
sudo ln -sf /etc/nginx/sites-available/apexintel /etc/nginx/sites-enabled/
sudo nginx -t && sudo systemctl reload nginx
```

---

## Architecture Overview

```
┌──────────────┐     ┌───────────┐     ┌──────────────┐
│   Frontend   │────▶│   Nginx   │────▶│   API Server │
│  (Next.js)   │     │  (proxy)  │     │  (Rust/Axum) │
└──────────────┘     └───────────┘     └──────┬───────┘
                                               │
                     ┌─────────────────────────┼─────────────────────────┐
                     │                         │                         │
              ┌──────▼──────┐          ┌───────▼──────┐          ┌──────▼──────┐
              │  PostgreSQL │          │    Redis     │          │    NATS     │
              │  (primary)  │          │   (cache)    │          │ (job queue) │
              └─────────────┘          └──────────────┘          └──────┬──────┘
                                                                        │
                                                                ┌───────▼──────┐
                                                                │    Worker    │
                                                                │(crawl/enrich)│
                                                                └───────┬──────┘
                                                                        │
                     ┌─────────────────────────┬────────────────────────┤
                     │                         │                        │
              ┌──────▼──────┐          ┌───────▼──────┐         ┌──────▼──────┐
              │    MinIO    │          │  LLM Server  │         │   Tantivy   │
              │ (artifacts) │          │ (llama-server)│         │   (search)  │
              └─────────────┘          └──────────────┘         └─────────────┘
```

### Crate Map

| Crate | Purpose | Key Modules |
|-------|---------|-------------|
| `api` | HTTP server, routes, auth | routes/*, rate_limit, filters |
| `worker` | Background jobs | scheduler, nightly, weekly, sla_predictor |
| `core` | Shared types, config | entities, schemas, quality_score, geospatial |
| `crawl` | Web scraping, feeds | rate_limit, social/*, rss, academic, diff_engine |
| `parse` | Content extraction | html, patent, cert, sentiment, transliteration |
| `graph` | Entity relationships | entity_resolution, edge_expiry, stale_pruner |
| `insights` | Intelligence generation | memo, dossier, cep, arbitrage, news_digest |
| `stats` | Statistical analysis | anomaly, changepoint, correlation, bayesian |
| `poi` | Person-of-interest | features, engagement_tracker, photo_detector |
| `store` | Database layer | PostgreSQL queries, migrations |
| `recipes` | Signal detection | Recipe engine, scoring |
| `llm` | LLM integration | Prompt building, response parsing |
| `learning` | ML features | Feature store, model serving |

---

## Daily Operations

### Morning Checklist (5 minutes)

1. **Dashboard**: Open `https://starzerp.fi` — verify data is recent
2. **Health**: `curl -s https://starzerp.fi/api/health/deep | jq .`
3. **Warnings**: Check `/warnings` page for unacknowledged critical/high alerts
4. **Admin**: Check `/admin` for crawler status and job health
5. **Logs**: Quick scan for errors:
   ```bash
   journalctl -u apexintel-api --since "8 hours ago" -p err --no-pager | tail -20
   journalctl -u apexintel-worker --since "8 hours ago" -p err --no-pager | tail -20
   ```

### Weekly Tasks

- **Monday**: Review recipe performance at `/recipes` — deprecate low-precision recipes
- **Wednesday**: Check data retention — verify old observations are being pruned
- **Friday**: Review POI engagement activities and update stale profiles

### Monthly Tasks

- **Backup verification**: Restore a backup to a test database and verify integrity
- **SSL renewal**: `sudo certbot renew --dry-run`
- **Dependency audit**: Check for Rust/npm security advisories
- **Performance review**: Check Grafana dashboards for degradation trends

---

## Common Failure Modes

| Symptom | Likely Cause | Diagnosis | Fix |
|---------|-------------|-----------|-----|
| API returns 500 | PostgreSQL pool exhausted | `psql -c "SELECT count(*) FROM pg_stat_activity"` | `systemctl restart apexintel-api` |
| No new warnings for 6+ hours | Worker crashed or hung | `systemctl status apexintel-worker` | `systemctl restart apexintel-worker` |
| LLM responses timeout | Model OOM or server crashed | `curl http://localhost:8081/health` | `systemctl restart apexintel-llm` |
| Crawl failure rate >5% | IP blocked by targets | Check `crawl_metrics` table error rates | Enable/rotate `SCRAPER_PROXY_URL` |
| Feature store stale | Nightly job failed | Check logs: `journalctl -u apexintel-worker --since yesterday` | Manual trigger: `POST /api/admin/trigger/feature-store` |
| WebSocket disconnects | Nginx proxy timeout | Check `proxy_read_timeout` in nginx config | Increase to 300s, reload nginx |
| Disk full | Logs or MinIO artifacts | `df -h` and `du -sh /var/log/*` | Rotate logs, clean old artifacts |
| High memory usage | Too many concurrent crawls | `htop` — check worker RSS | Reduce `CRAWL_CONCURRENCY` |
| Frontend build fails | npm dependency issue | `cd frontend && npm ci` | Delete `node_modules`, reinstall |
| Migration fails | Schema already exists | Check `\dt` in psql | Use `IF NOT EXISTS` or skip |

---

## Recovery Procedures

### Database Corruption / Data Loss

```bash
# 1. Stop services
sudo systemctl stop apexintel-api apexintel-worker

# 2. List available backups
ls -la /opt/apexintel/backups/

# 3. Restore from latest backup
./scripts/restore.sh /opt/apexintel/backups/LATEST

# 4. Verify data integrity
psql "$DATABASE_URL" -c "SELECT count(*) FROM entities;"
psql "$DATABASE_URL" -c "SELECT count(*) FROM observations;"

# 5. Restart services
sudo systemctl start apexintel-api apexintel-worker
```

### Full System Restart

```bash
# Graceful restart of all services
sudo systemctl restart apexintel-{api,worker,frontend,llm}

# Verify all healthy
for svc in api worker frontend llm; do
  echo "=== apexintel-$svc ==="
  sudo systemctl is-active apexintel-$svc
done

# Wait 10 seconds, then health check
sleep 10
curl -s http://localhost:8080/api/health/deep | jq .
```

### Recipe Engine Issues

```bash
# Force recipe reload from database
curl -X POST http://localhost:8080/api/admin/trigger/recipe-reload \
  -H "Authorization: Bearer $ADMIN_TOKEN"

# Check recipe count
curl -s http://localhost:8080/api/recipes | jq '.total'
```

### POI Profiles Outdated

```bash
# Force POI refresh
curl -X POST http://localhost:8080/api/admin/trigger/poi-refresh \
  -H "Authorization: Bearer $ADMIN_TOKEN"

# Monitor progress
journalctl -u apexintel-worker -f | grep -i poi
```

### Tantivy Index Corruption

```bash
# Stop API
sudo systemctl stop apexintel-api

# Remove corrupted index
rm -rf /opt/apexintel/data/tantivy_index/

# Restart — index will rebuild from database
sudo systemctl start apexintel-api

# Monitor rebuild progress
journalctl -u apexintel-api -f | grep -i tantivy
```

### Redis Cache Issues

```bash
# Flush all cache (safe — will rebuild)
redis-cli -u "$REDIS_URL" FLUSHALL

# Verify connection
redis-cli -u "$REDIS_URL" PING
```

---

## Performance Tuning

### PostgreSQL (production settings)

```sql
-- /etc/postgresql/16/main/postgresql.conf
-- Adjust based on available RAM (example for 32GB server)

shared_buffers = '8GB'              -- 25% of RAM
effective_cache_size = '24GB'       -- 75% of RAM
work_mem = '256MB'                  -- per-operation memory
maintenance_work_mem = '1GB'        -- for VACUUM, CREATE INDEX
wal_buffers = '64MB'
checkpoint_completion_target = 0.9
random_page_cost = 1.1              -- for NVMe SSD
effective_io_concurrency = 200      -- for NVMe SSD
max_connections = 200
max_parallel_workers_per_gather = 4
max_parallel_workers = 8
```

### Worker Concurrency

```bash
# In .env
CRAWL_CONCURRENCY=8          # parallel crawl tasks (default: 4)
ENRICHMENT_CONCURRENCY=4     # parallel LLM enrichment (depends on VRAM)
NIGHTLY_TIMEOUT_SECS=3600    # 1 hour max for nightly jobs
WEEKLY_TIMEOUT_SECS=7200     # 2 hours max for weekly jobs
```

### LLM Server

```bash
# In apexintel-llm.service
# Adjust --parallel based on available VRAM
# Each parallel slot uses ~3GB VRAM
ExecStart=/opt/apexintel/llama-server \
  --model /opt/apexintel/models/apex-intel.gguf \
  --host 127.0.0.1 \
  --port 8081 \
  --parallel 4 \           # 4 concurrent requests (needs ~12GB VRAM)
  --ctx-size 4096 \        # context window
  --batch-size 512 \       # batch processing
  --threads 8              # CPU threads for prompt processing
```

### Redis

```bash
# Monitor memory usage
redis-cli -u "$REDIS_URL" INFO memory

# Set eviction policy for cache
redis-cli -u "$REDIS_URL" CONFIG SET maxmemory 2gb
redis-cli -u "$REDIS_URL" CONFIG SET maxmemory-policy allkeys-lru
```

### Nginx

```nginx
# Key performance settings
worker_processes auto;
worker_connections 4096;

# Enable gzip for API responses
gzip on;
gzip_types application/json text/plain text/css application/javascript;
gzip_min_length 1000;

# Increase buffer sizes for large API responses
proxy_buffer_size 128k;
proxy_buffers 4 256k;
proxy_busy_buffers_size 256k;
```

---

## Scaling Guidelines

### Vertical Scaling (single server)

| Component | Minimum | Recommended | High Load |
|-----------|---------|-------------|-----------|
| CPU | 4 cores | 8-16 cores | 32+ cores |
| RAM | 16GB | 32-64GB | 128GB+ |
| Storage | 200GB NVMe | 500GB NVMe | 1TB+ NVMe |
| GPU (LLM) | None (CPU) | RTX 4090 (24GB) | A100 (80GB) |

### Horizontal Scaling

```
Phase 1: Single Server (current)
├── API + Worker + Frontend + PostgreSQL + Redis + NATS

Phase 2: Separate Database (10-50 users)
├── App Server: API + Worker + Frontend
└── DB Server: PostgreSQL + Redis

Phase 3: Full Separation (50-200 users)
├── API Server(s): API + Frontend (behind load balancer)
├── Worker Server(s): Worker instances (NATS job claiming)
├── DB Server: PostgreSQL primary + read replica
├── Cache Server: Redis Sentinel
└── LLM Server: Dedicated GPU node

Phase 4: Kubernetes (200+ users)
├── k8s cluster with auto-scaling
├── PostgreSQL: CrunchyData operator
├── Redis: Redis operator
├── NATS: NATS operator
└── LLM: Dedicated GPU node pool
```

### Multiple Worker Instances

Workers use NATS-based job claiming (at-most-once delivery), so multiple workers are safe:

```bash
# Start 3 worker instances
for i in 1 2 3; do
  WORKER_ID=$i ./target/release/apex-worker &
done
```

### Read Replicas

```bash
# Add replica connection string for read-heavy API queries
DATABASE_READ_URL=postgres://user:pass@replica:5432/apexintel

# API server will route SELECT queries to replica
```

### CDN / Static Assets

```bash
# Put frontend behind Cloudflare
# 1. Add domain to Cloudflare
# 2. Enable "Full (strict)" SSL mode
# 3. Add page rules for caching static assets
# 4. Enable Brotli compression
```

---

## Security Checklist

### Initial Hardening

- [ ] Change default JWT secret (`openssl rand -hex 32`)
- [ ] Set strong PostgreSQL passwords
- [ ] Enable `SCRAPER_PROXY_URL` for production crawling
- [ ] Configure firewall (only 80/443 open externally)
- [ ] Disable SSH password auth (key-only)
- [ ] Set up fail2ban for SSH and Nginx
- [ ] Enable PostgreSQL SSL (`sslmode=require`)
- [ ] Set Redis password and bind to localhost only
- [ ] Review API rate limiting tiers in `crates/api/src/rate_limit.rs`

### Ongoing Security

- [ ] Rotate JWT secret quarterly
- [ ] Update SSL certificates before expiry
- [ ] Monitor `/api/health/deep` for component health
- [ ] Review API access logs weekly for anomalies
- [ ] Keep Rust toolchain and npm dependencies updated
- [ ] Run `cargo audit` and `npm audit` monthly

---

## Incident Playbook

### Severity Levels

| Level | Definition | Response Time | Example |
|-------|-----------|---------------|---------|
| P1 - Critical | System down, data loss | 15 min | Database corruption, API completely unresponsive |
| P2 - High | Major feature broken | 1 hour | Crawling stopped, LLM unavailable |
| P3 - Medium | Feature degraded | 4 hours | Slow queries, elevated error rate |
| P4 - Low | Cosmetic / minor | Next business day | UI glitch, non-critical log errors |

### Incident Response Steps

1. **Detect**: Automated alert (Grafana) or user report
2. **Acknowledge**: Assign responder, update status page
3. **Diagnose**: Check health endpoint, logs, metrics
4. **Contain**: Isolate failing component (restart, disable feature flag)
5. **Fix**: Apply the appropriate recovery procedure
6. **Verify**: Health check, smoke test critical paths
7. **Post-mortem**: Document what happened, timeline, and prevention

### Example: P1 — API Completely Down

```bash
# 1. Quick check
curl http://localhost:8080/api/health  # Timeout? → proceed

# 2. Check systemd
sudo systemctl status apexintel-api   # Active? Dead?

# 3. Check logs for root cause
journalctl -u apexintel-api -n 50 --no-pager

# 4. Common fixes
# a) OOM killed → increase memory limit or reduce connections
# b) Port conflict → check `ss -tlnp | grep 8080`
# c) Config error → check .env file
# d) Database down → `sudo systemctl status postgresql`

# 5. Restart
sudo systemctl restart apexintel-api

# 6. Verify
sleep 5
curl -s http://localhost:8080/api/health | jq .
```

### Example: P2 — No New Data for 12+ Hours

```bash
# 1. Check worker
sudo systemctl status apexintel-worker
journalctl -u apexintel-worker --since "12 hours ago" -p err

# 2. Check recent observations
psql "$DATABASE_URL" -c "SELECT count(*), max(observed_at) FROM observations WHERE observed_at > now() - interval '24 hours';"

# 3. Check NATS queue
# If queue is full → workers not consuming
# If queue is empty → scheduler not producing

# 4. Check external connectivity
curl -s https://www.google.com -o /dev/null -w "%{http_code}"  # Should be 200

# 5. Restart worker
sudo systemctl restart apexintel-worker
```

---

## Monitoring & Alerting

### Grafana Dashboard

Import the dashboard from `config/grafana/apexintel-dashboard.json`. Key panels:

| Panel | Alert Threshold |
|-------|----------------|
| API Request Rate | < 1 req/min for 10 min → P2 |
| API Error Rate | > 5% for 5 min → P2 |
| Worker Job Failures | > 10% for 15 min → P2 |
| Database Connections | > 80% pool → P3 |
| Disk Usage | > 85% → P3, > 95% → P1 |
| Memory Usage | > 90% → P3 |
| Crawl Success Rate | < 90% for 30 min → P3 |
| LLM Response Time | p95 > 30s → P3 |

### Health Check Endpoints

```bash
# Basic health (fast, for load balancer)
GET /api/health
# → {"status":"ok","version":"0.1.0","uptime_secs":86400}

# Deep health (checks all dependencies)
GET /api/health/deep
# → {"postgres":"ok","redis":"ok","nats":"ok","minio":"ok","llm":"ok","tantivy":"ok"}
```

### Log Aggregation

```bash
# Structured JSON logs — pipe to your log aggregator
journalctl -u apexintel-api -o json | your-log-shipper

# Quick error count by hour
journalctl -u apexintel-api --since "24 hours ago" -p err --output=short | \
  awk '{print $1, $2, $3}' | cut -d: -f1 | sort | uniq -c | sort -rn
```

### Backup Schedule

```bash
# Automated via cron (see scripts/backup.sh)
# Default schedule: daily at 02:00 UTC
0 2 * * * /opt/apexintel/scripts/backup.sh >> /var/log/apexintel-backup.log 2>&1

# Retention: 7 daily + 4 weekly + 3 monthly
# Verify: ls -la /opt/apexintel/backups/
```

---

## Appendix: Environment Variables Reference

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `DATABASE_URL` | Yes | — | PostgreSQL connection string |
| `REDIS_URL` | Yes | `redis://127.0.0.1:6379` | Redis connection string |
| `NATS_URL` | Yes | `nats://127.0.0.1:4222` | NATS connection string |
| `MINIO_ENDPOINT` | Yes | `http://127.0.0.1:9000` | MinIO endpoint |
| `MINIO_ACCESS_KEY` | Yes | — | MinIO access key |
| `MINIO_SECRET_KEY` | Yes | — | MinIO secret key |
| `JWT_SECRET` | Yes | — | JWT signing secret (hex-encoded) |
| `LLM_SERVER_URL` | No | `http://127.0.0.1:8081` | LLM server URL |
| `SCRAPER_USER_AGENT` | No | `ApexIntel/1.0` | Crawler user agent |
| `SCRAPER_PROXY_URL` | No | — | HTTP proxy for crawling |
| `CRAWL_CONCURRENCY` | No | `4` | Max parallel crawl tasks |
| `API_PORT` | No | `8080` | API server port |
| `FRONTEND_PORT` | No | `3000` | Next.js port |
| `LOG_LEVEL` | No | `info` | Log level (trace/debug/info/warn/error) |
| `BACKUP_DIR` | No | `/opt/apexintel/backups` | Backup directory |
| `DATA_RETENTION_DAYS` | No | `365` | Days before observation archival |

---

*This runbook is a living document. Update it whenever infrastructure changes or new failure modes are discovered.*
