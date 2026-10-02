# ApexIntel – Production Deployment Guide

**Audience**: DevOps, System Administrator  
**Revision**: March 2026  
**Status**: 🚀 Deployed (multi-service: `apexintel-api`, `apexintel-worker`, `apexintel-llm`)  
**Domain**: https://<PRODUCTION_DOMAIN>

---

## 0. Target Production Environment

| Item | Value |
|------|-------|
| **Domain** | `https://<PRODUCTION_DOMAIN>` |
| **VPS Provider** | Hetzner |
| **VPS IPv4** | `<PRODUCTION_HOST>` |
| **OS** | Ubuntu 24.04 LTS (aarch64 / ARM64) |
| **Arch** | Ampere Altra (Neoverse-N1) |
| **SSH User** | `<DEPLOY_USER>` |
| **Colocated with** | CRM-v2 (Starz Morocco CRM) — **completely separate** |

### 0.1 SSH Key Setup

| Item | Value |
|------|-------|
| **Private key** | `~/.ssh/<DEPLOY_KEY>` (local machine) |
| **Public key** | `~/.ssh/<DEPLOY_KEY>.pub` |

```bash
ssh -i ~/.ssh/<DEPLOY_KEY> <DEPLOY_USER>@<PRODUCTION_HOST>
```

---

### 0.2 Application Architecture

ApexIntel runs as **separate Rust services**, not a single binary:
- **`apexintel-api`** (`apex-api`) serves:
  - **HTML pages** via Askama templates + HTMX (no JavaScript framework)
  - **REST API** (`/api/*`) with JSON responses
  - **WebSocket** (`/ws/*`) for live updates
  - **API-served assets** (CSS, JS, fonts, icons also mirrored for nginx)
- **`apexintel-worker`** (`apex-worker`) runs the background crawl/insight/SLA pipeline.
- **`apexintel-llm`** (llama-server) serves the local model on port 8081.

There is **no Node.js frontend**. The web UI templates are compiled into the
`apex-api` binary, but the API and the worker are deployed, restarted, and
versioned independently.

### 0.3 Application Stack

| Component | Version | Purpose |
|-----------|---------|---------|
| **Rust** | 1.80+ | API server + Worker (compiled binaries) |
| **PostgreSQL** | 16+ | Primary database |
| **Redis** | 7+ | Cache + rate limiting |
| **NATS** | 2.10+ | Message queue (inter-crate async messaging) |
| **MinIO** | Latest | S3-compatible object storage |
| **llama-server** | Latest (llama.cpp) | LLM inference for trained Qwen3-30B-A3B |
| **Nginx** | 1.24+ | Reverse proxy + TLS termination + static file serving |
| **Certbot** | Latest | Let's Encrypt SSL auto-renewal |

> **Note**: Node.js is **not required**. The Next.js frontend was fully replaced by
> server-rendered Askama/HTMX templates compiled into the `apex-api` binary (March
> 2026); static assets are served by nginx from disk.

### 0.4 Separation from CRM-v2

| Resource | CRM-v2 | ApexIntel |
|----------|--------|-----------|
| **App directory** | `/var/www/crm-starz-morocco/` | `/opt/apexintel/` |
| **Nginx vhost** | `/etc/nginx/sites-available/starzcrm` | `/etc/nginx/sites-available/apexintel` |
| **Database** | MySQL `starz_crm` | PostgreSQL `apexintel` |
| **Systemd services** | `starz-messenger` | `apexintel-api`, `apexintel-worker`, `apexintel-llm` |
| **Ports (internal)** | PHP-FPM socket | API: 8080, LLM: 8081, NATS: 4222, MinIO: 9000, Redis: 6379, PG: 5432 |
| **Domain** | starzcrm.com | https://<PRODUCTION_DOMAIN> |
| **User** | www-data | apexintel |

---

## 1. Server Setup (Fresh)

### 1.1 System Requirements

**Minimum**: 8 vCPU, 32 GB RAM, 200 GB SSD, 1 Gbps NIC  
**Recommended for LLM**: 16+ vCPU, 64+ GB RAM (Qwen3-30B-A3B Q4_K_M needs ~17 GB RAM)  
**Network**: Ports 22, 80, 443 open inbound; all outbound open

### 1.2 Hetzner Cloud Firewall

| Protocol | Port | Source | Purpose |
|----------|------|--------|---------|
| TCP | 22 | Any (or your IP) | SSH access |
| TCP | 80 | Any | HTTP (Certbot + redirect) |
| TCP | 443 | Any | HTTPS (production traffic) |

### 1.3 Create System User

```bash
useradd -r -m -d /opt/apexintel -s /bin/bash apexintel
mkdir -p /opt/apexintel/{bin,data,logs,model,config,static}
chown -R apexintel:apexintel /opt/apexintel
# Allow nginx (www-data) to traverse to /opt/apexintel/static
chmod o+x /opt/apexintel
```

### 1.4 Install System Packages

```bash
sudo apt update && sudo apt upgrade -y
sudo apt install -y \
  build-essential pkg-config libssl-dev \
  postgresql postgresql-contrib \
  redis-server \
  nginx certbot python3-certbot-nginx \
  curl wget git unzip htop jq \
  cmake gcc g++ \
  chromium fonts-liberation
```

### 1.5 Install Rust (only needed if building on server)

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source $HOME/.cargo/env
rustup default stable
```

### 1.6 Install NATS Server

```bash
curl -sf https://binaries.nats.dev/nats-io/nats-server/v2@latest | sh
sudo mv nats-server /usr/local/bin/
```

### 1.7 Install MinIO

```bash
# For ARM64 (Hetzner Ampere):
wget https://dl.min.io/server/minio/release/linux-arm64/minio
chmod +x minio
sudo mv minio /usr/local/bin/
mkdir -p /opt/apexintel/data/minio
```

### 1.8 Install llama.cpp (llama-server)

```bash
cd /tmp
git clone https://github.com/ggerganov/llama.cpp.git
cd llama.cpp
cmake -B build -DGGML_BLAS=ON -DGGML_BLAS_VENDOR=OpenBLAS
cmake --build build --config Release -j$(nproc)
sudo cp build/bin/llama-server /usr/local/bin/
```

> For CPU-only inference, OpenBLAS provides adequate performance.
> If the server has a GPU, use `-DGGML_CUDA=ON` instead.

---

## 2. Database Setup

### 2.1 PostgreSQL

```bash
# Generate one secure password per role:
openssl rand -base64 32   # → DB_PASSWORD (apexintel_app)
openssl rand -base64 32   # → MIGRATOR_PASSWORD (apexintel_migrator)

# Create the two roles and the database (as the postgres superuser):
sudo -u postgres psql <<'SQL'
-- Owner/migration role: owns the database and every migration-created object.
CREATE ROLE apexintel_migrator LOGIN PASSWORD '<MIGRATOR_PASSWORD>';

-- Runtime role: least privilege, no DDL, cannot bypass RLS.
CREATE ROLE apexintel_app LOGIN PASSWORD '<DB_PASSWORD>'
    NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS;

CREATE DATABASE apexintel OWNER apexintel_migrator;

\c apexintel
CREATE EXTENSION IF NOT EXISTS "uuid-ossp";
CREATE EXTENSION IF NOT EXISTS "pg_trgm";

-- Runtime privileges: connect + DML only (no DDL)
GRANT CONNECT ON DATABASE apexintel TO apexintel_app;
GRANT USAGE ON SCHEMA public TO apexintel_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO apexintel_app;
GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO apexintel_app;

-- Objects created later by the migrator inherit the same runtime grants,
-- so migrations applied at API startup need no follow-up GRANTs:
ALTER DEFAULT PRIVILEGES FOR ROLE apexintel_migrator IN SCHEMA public
    GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO apexintel_app;
ALTER DEFAULT PRIVILEGES FOR ROLE apexintel_migrator IN SCHEMA public
    GRANT USAGE, SELECT ON SEQUENCES TO apexintel_app;
SQL
```

**Connection strings**:
- Runtime (`apex-api` + `apex-worker`): `postgresql://apexintel_app:<DB_PASSWORD>@127.0.0.1:5432/apexintel`
- Migrations (API startup path): `postgresql://apexintel_migrator:<MIGRATOR_PASSWORD>@127.0.0.1:5432/apexintel`

**Role contract**: `apexintel_migrator` owns the `apexintel` database and applies
migrations (DDL allowed). `apexintel_app` is the runtime role used by both the API
and the worker; it has no DDL rights. If a migration creates objects owned by
another role, re-run the `GRANT` statements above after applying it.

### 2.2 Redis

```bash
sudo systemctl enable redis-server
sudo systemctl start redis-server
redis-cli ping  # → PONG
```

---

## 3. Application Deployment

### 3.1 File System Layout

```
/opt/apexintel/                          ← Application root (owner: apexintel)
├── bin/
│   ├── apex-api                         ← Rust API + web UI binary (service: apexintel-api)
│   └── apex-worker                      ← Rust background worker binary (service: apexintel-worker)
├── config/
│   └── .env                             ← Production environment (secrets + auth)
├── data/
│   ├── search/                          ← Tantivy full-text index
│   └── minio/                           ← MinIO object storage data
├── static/                              ← Static assets (served by nginx)
│   ├── css/
│   │   ├── tailwind.css                 ← Compiled Tailwind CSS
│   │   └── globals.css                  ← Custom styles
│   ├── js/
│   │   ├── htmx.min.js                 ← HTMX library
│   │   ├── app.js                       ← App-level JS (theme, search, sidebar)
│   │   ├── graph.js                     ← D3 graph visualization
│   │   └── recipe-builder.js            ← Recipe pipeline builder
│   ├── fonts/
│   │   └── InterVariable*.woff2/ttf     ← Inter font family
│   └── icons/
│       └── sprite.svg                   ← SVG icon sprite
├── model/                               ← LLM model weights (GGUF)
│   └── Qwen3-30B-A3B-Q4_K_M.gguf       ← Quantized model (~17 GB)
└── logs/
    ├── api.log
    ├── worker.log
    └── llm.log
```

### 3.2 Build & Deploy (Recommended: Cross-compile locally)

The preferred method is cross-compiling on your local machine and uploading the binaries:

```bash
# On local machine (macOS with cargo-zigbuild):
cd <REPO_ROOT>

# Install cross-compilation tools (one time):
cargo install cargo-zigbuild
brew install zig  # or equivalent for your OS

# Build release binaries for ARM64 Linux:
cargo zigbuild --release --target aarch64-unknown-linux-gnu -p apex-api --features llm --locked
cargo zigbuild --release --target aarch64-unknown-linux-gnu -p apex-worker --features llm --locked

# Upload binaries:
scp -i ~/.ssh/<DEPLOY_KEY> \
  target/aarch64-unknown-linux-gnu/release/apex-api \
  <DEPLOY_USER>@<PRODUCTION_HOST>:/opt/apexintel/bin/apex-api
scp -i ~/.ssh/<DEPLOY_KEY> \
  target/aarch64-unknown-linux-gnu/release/apex-worker \
  <DEPLOY_USER>@<PRODUCTION_HOST>:/opt/apexintel/bin/apex-worker

# Upload static assets:
scp -i ~/.ssh/<DEPLOY_KEY> -r \
  crates/api/static/* \
  <DEPLOY_USER>@<PRODUCTION_HOST>:/opt/apexintel/static/

# Set permissions on server:
ssh -i ~/.ssh/<DEPLOY_KEY> <DEPLOY_USER>@<PRODUCTION_HOST> \
  "chmod 700 /opt/apexintel/bin/apex-api /opt/apexintel/bin/apex-worker && \
   chown -R apexintel:apexintel /opt/apexintel/bin /opt/apexintel/static && \
   systemctl restart apexintel-api apexintel-worker"
```

### 3.3 Build on Server (Alternative)

```bash
cd /opt/apexintel/src
cargo build --release -p apex-api --features llm --locked
cargo build --release -p apex-worker --features llm --locked
cp target/release/apex-api /opt/apexintel/bin/
cp target/release/apex-worker /opt/apexintel/bin/
chown apexintel:apexintel /opt/apexintel/bin/apex-api /opt/apexintel/bin/apex-worker
```

### 3.4 Database Migrations

Migrations are embedded in the `apex-store` crate and run automatically when the API and worker start (`store.run_migrations().await`). No manual migration step is needed, and neither process will start against an unknown schema: migration failure aborts startup. `APEX_SKIP_MIGRATIONS=1` is only honored after verifying that the applied history matches the embedded migrations (latest version and every checksum), so a binary update against a lagging DB must be preceded by the schema preflight in §8 "Routine Update".

---

### 3.5 Release evidence — a green SHA is only claimed with its bundle

A commit is **not** a green release just because the tests happened to pass on
someone's machine. The release contract is the retained evidence bundle
produced by `scripts/ci/release_evidence.sh` for the exact SHA:

```bash
# Run the full canonical matrix for HEAD and retain the bundle.
scripts/ci/release_evidence.sh --sha "$(git rev-parse HEAD)"
# -> release-evidence/<sha>/release-evidence.json
#    release-evidence/<sha>/release-evidence.txt
#    release-evidence/<sha>/release-evidence.sha256
#    release-evidence/<sha>/logs/<gate>.log
```

The bundle is only authoritative when:

- `git_sha` equals the deployed commit and `exact_sha` is `true` (HEAD == SHA
  and the worktree is clean at run time);
- `status` is `passed` (no gate failed) and `complete` is `true` (every
  canonical gate ran or was recorded by its owning pipeline step);
- `artifact_digest` matches the deployed binary, and `/api/version` on the
  running service reports the same `git_sha`, `build_timestamp` and
  `artifact_digest`.

Gates that were never executed and never recorded are published as `not_run`,
so a partial bundle can never claim a gate that did not happen. Where CI owns
a gate in its own pipeline step, record it explicitly
(`--record <gate>=passed`) only after that step succeeded. Absent a bundle
whose JSON says `status=passed` and `complete=true`, the SHA must be treated
as unverified and must not be deployed as a release.

---

## 4. Environment Configuration

Create `/opt/apexintel/config/.env`:

```dotenv
# ══════════════════════════════════════════════════════════════════════════════
# ApexIntel Production Environment Configuration
# ══════════════════════════════════════════════════════════════════════════════
# ⚠️ SECURITY: chmod 600, owned by apexintel. Never commit to VCS.
# ══════════════════════════════════════════════════════════════════════════════

# ─── Database ─────────────────────────────────────────────────────────────────
# Two roles (see section 2.1):
#   apexintel_app      — runtime role for apex-api and apex-worker (no DDL rights)
#   apexintel_migrator — database owner; used only to apply migrations
# Generate passwords: openssl rand -base64 32
DATABASE_URL=postgresql://apexintel_app:<DB_PASSWORD>@127.0.0.1:5432/apexintel
MIGRATION_DATABASE_URL=postgresql://apexintel_migrator:<MIGRATOR_PASSWORD>@127.0.0.1:5432/apexintel

# ─── Service URLs ─────────────────────────────────────────────────────────────
REDIS_URL=redis://127.0.0.1:6379
NATS_URL=nats://127.0.0.1:4222
MINIO_URL=http://127.0.0.1:9000
MINIO_BUCKET=apexintel

# ─── LLM ──────────────────────────────────────────────────────────────────────
LLM_BASE_URL=http://127.0.0.1:8081
LLM_MODEL=Qwen3-30B-A3B-Q4_K_M

# ─── API Server ───────────────────────────────────────────────────────────────
HOST=127.0.0.1
PORT=8080
CORS_ORIGIN=https://<PRODUCTION_DOMAIN>
API_LOG_LEVEL=info
SEARCH_INDEX_PATH=/opt/apexintel/data/search

# ─── Web Authentication (built-in login) ─────────────────────────────────────
# The API binary serves the login page and handles session cookies directly.
# Username: choose an admin username
APEX_ADMIN_USERNAME=<YOUR_ADMIN_USERNAME>
# Password hash: Argon2id PHC string, single-quoted for dotenvy.
# Generate in-repo: cargo run -p apex-api --example hash_password -- 'yourpassword'
# Or with the argon2 CLI: echo -n 'yourpassword' | argon2 "$(openssl rand -hex 16)" -id -e
# Legacy SHA-256 hashes are REJECTED unless ALLOW_LEGACY_PASSWORD_HASHES=true.
# That escape hatch exists only for explicitly-enabled upgrades of old
# deployments — keep it false in production (Argon2id PHC hashes only).
ALLOW_LEGACY_PASSWORD_HASHES=false
APEX_ADMIN_PASSWORD_HASH='$argon2id$v=19$m=19456,t=2,p=1$<SALT>$<HASH>'
# Session secret: openssl rand -hex 32
SESSION_SECRET=<GENERATE_64_CHAR_HEX_SECRET>
# Optional multi-user login (takes precedence over APEX_ADMIN_*):
# JSON array of {id, username, password_hash, role}; role: admin|analyst|viewer|service
# WEB_USERS_JSON='[{"id":"usr-admin","username":"admin","password_hash":"$argon2id$...","role":"admin"}]'

# ─── API Key Authentication (for external API consumers) ─────────────────────
# Generate: openssl rand -hex 32 | sed 's/^/sk-apex-/'
# Format: <raw_key>,<display_name>,<role>[,<user_id>]
# Roles: admin, analyst, viewer, service
API_KEY_1=<GENERATE_API_KEY>,Production Admin,admin,usr-production-admin

# ─── Crawl & Scheduling ──────────────────────────────────────────────────────
CRAWL_INTERVAL_SECS=21600
NIGHTLY_HOUR_UTC=2
WEEKLY_DAY=0
DEFAULT_RPS=0.2
PROXY_POOL_SIZE=50
ENABLE_PROXY_ROTATION=false

# Headless browser rendering (single persistent Chromium/CDP renderer).
# The full profile renders Browser-strategy sources, so this is enabled and
# points at the Chromium installed in section 1.4 (Dockerfile.worker ships
# Chromium and sets both variables itself). With ENABLE_HEADLESS_BROWSER=false
# Browser-strategy sources are reported "Unavailable: missing capability" and
# are never silently downgraded to plain HTTP.
# Chromium MUST run as the dedicated non-root "apexintel" service account:
# the renderer deliberately does NOT pass --no-sandbox, so the sandbox needs a
# non-privileged user (the shipped Dockerfile.api/Dockerfile.worker images
# already create and switch to "apexintel"). Never run this as root and never
# reintroduce --no-sandbox.
ENABLE_HEADLESS_BROWSER=true
HEADLESS_BROWSER_BIN=/usr/bin/chromium
# HEADLESS_BROWSER_MAX_CONCURRENCY=2            # global browser ceiling (max 2)
# HEADLESS_BROWSER_TIMEOUT_SECS=30              # hard total render-time cap
# HEADLESS_BROWSER_QUIET_WINDOW_MS=1000         # network-idle window, clamped 750-1500
# HEADLESS_BROWSER_SCROLL_STEPS=3               # bounded progressive scroll for lazy content
# HTTP crawling runs 8-16 requests in parallel (default 12) with per-domain
# rate limiting; the browser fleet is limited to 1 process / 2 contexts.

# ─── Optional integrations ───────────────────────────────────────────────────
# GOOGLE_API_KEY=
# GOOGLE_SEARCH_ENGINE_ID=
# NEXAR_CLIENT_ID=
# NEXAR_CLIENT_SECRET=
# MOUSER_API_KEY=
# DIGIKEY_CLIENT_ID=
# SMTP_URL=
```

Set permissions:
```bash
chmod 600 /opt/apexintel/config/.env
chown apexintel:apexintel /opt/apexintel/config/.env
```

### 4.1 Environment Variables Reference

| Variable | Required | Description |
|----------|----------|-------------|
| `DATABASE_URL` | ✅ | Runtime PostgreSQL connection string (`apexintel_app` role; no DDL rights) |
| `MIGRATION_DATABASE_URL` | ✅ | Migration connection string (`apexintel_migrator` role); used by the API startup path to apply migrations |
| `REDIS_URL` | ✅ | Redis connection URL |
| `NATS_URL` | ✅ | NATS messaging URL |
| `HOST` / `PORT` | ✅ | Bind address (127.0.0.1:8080) |
| `CORS_ORIGIN` | ✅ | Allowed CORS origin (`https://<PRODUCTION_DOMAIN>`) |
| `APEX_ADMIN_USERNAME` | ✅ | Web login username |
| `APEX_ADMIN_PASSWORD_HASH` | ✅ | Argon2id PHC hash of the web login password |
| `ALLOW_LEGACY_PASSWORD_HASHES` | ➖ | `false` (recommended): legacy SHA-256 hashes are rejected; set `true` only for explicitly-enabled upgrades (Argon2id PHC hashes only in production) |
| `WEB_USERS_JSON` | ➖ | Optional multi-user login: JSON array of `{id, username, password_hash, role}`; overrides `APEX_ADMIN_*` |
| `SESSION_SECRET` | ✅ | 64-char hex for HMAC cookie signing |
| `COOKIE_SECURE` | ➖ | Set `1` behind HTTPS: session cookie becomes `__Host-apex_session` with `Secure` |
| `API_KEY_1` | ✅ | API key for external consumers |
| `LLM_BASE_URL` | ✅ | Local LLM inference endpoint |
| `SEARCH_INDEX_PATH` | ✅ | Tantivy index directory |
| `LLM_TIMEOUT_SECS` | ➖ | Per-call LLM timeout; code default 180, production sets `600` for 30B CPU inference |
| `APEX_GIT_SHA` | ✅ (prod) | Commit the running binaries were built from; surfaced by `/api/version` |
| `APEX_CI_PIPELINE_ID` | ✅ (prod) | Woodpecker pipeline that produced the artifact; surfaced by `/api/version` |
| `APEX_ARTIFACT_DIGEST` | ✅ (prod) | `sha256:<hex>` digest of the deployed artifact; surfaced by `/api/version` |
| `APEX_DEPLOYED_AT` | ✅ (prod) | RFC 3339 deploy timestamp; surfaced by `/api/version` |

> Provenance is not optional in production: `scripts/ops/record_deployment.sh`
> writes the per-deployment audit record and prints the four `APEX_*` exports
> for the service `.env`. Without `APEX_GIT_SHA`, the `deployment` capability in
> `/api/health/capabilities` reports `degraded`.
>
> `/api/version` is **public by design** (like `/api/health`) so load balancers
> and operators can verify the running build without credentials. If your
> threat model treats exact build identity as sensitive, allowlist the route at
> nginx or require a session in front of it.

---

## 5. Systemd Services

### 5.1 NATS Server

`/etc/systemd/system/nats.service`:

```ini
[Unit]
Description=NATS Message Server
After=network.target

[Service]
Type=simple
User=apexintel
ExecStart=/usr/local/bin/nats-server -a 127.0.0.1 -p 4222
Restart=on-failure
RestartSec=5
LimitNOFILE=65536

[Install]
WantedBy=multi-user.target
```

### 5.2 MinIO Object Storage

`/etc/systemd/system/minio.service`:

```ini
[Unit]
Description=MinIO Object Storage
After=network.target

[Service]
Type=simple
User=apexintel
Environment="MINIO_ROOT_USER=apexintel"
Environment="MINIO_ROOT_PASSWORD=<GENERATE_MINIO_PASSWORD>"
ExecStart=/usr/local/bin/minio server /opt/apexintel/data/minio --address 127.0.0.1:9000 --console-address 127.0.0.1:9001
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
```

### 5.3 ApexIntel API Server (serves web UI + API)

`/etc/systemd/system/apexintel-api.service`:

```ini
[Unit]
Description=ApexIntel API Server (Rust/Axum — serves HTML + API)
After=network.target postgresql.service redis.service nats.service minio.service
Requires=postgresql.service

[Service]
Type=simple
User=apexintel
WorkingDirectory=/opt/apexintel
EnvironmentFile=/opt/apexintel/config/.env
ExecStart=/opt/apexintel/bin/apex-api
Restart=on-failure
RestartSec=5
LimitNOFILE=65536

# Security hardening
ProtectSystem=strict
ReadWritePaths=/opt/apexintel/data /opt/apexintel/logs
ReadOnlyPaths=/opt/apexintel/static

# Logging
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=multi-user.target
```

### 5.4 ApexIntel Worker

`/etc/systemd/system/apexintel-worker.service`:

```ini
[Unit]
Description=ApexIntel Background Worker (Nightly + Weekly pipelines)
After=network.target postgresql.service redis.service nats.service
Requires=postgresql.service

[Service]
Type=simple
User=apexintel
WorkingDirectory=/opt/apexintel
EnvironmentFile=/opt/apexintel/config/.env
ExecStart=/opt/apexintel/bin/apex-worker
Restart=on-failure
RestartSec=10
LimitNOFILE=65536

StandardOutput=journal
StandardError=journal

[Install]
WantedBy=multi-user.target
```

### 5.5 LLM Inference Server (llama-server)

`/etc/systemd/system/apexintel-llm.service`:

```ini
[Unit]
Description=ApexIntel LLM Inference (llama-server / Qwen3-30B-A3B)
After=network.target

[Service]
Type=simple
User=apexintel
ExecStart=/usr/local/bin/llama-server \
  --model /opt/apexintel/model/Qwen3-30B-A3B-Q4_K_M.gguf \
  --host 127.0.0.1 \
  --port 8081 \
  --ctx-size 4096 \
  --threads 8 \
  --batch-size 512 \
  --parallel 2 \
  --mlock
Restart=on-failure
RestartSec=10
LimitNOFILE=65536

StandardOutput=journal
StandardError=journal

[Install]
WantedBy=multi-user.target
```

> **Warning-analysis context:** the grounded warning analysis (`POST /api/warnings/:id/analysis`)
> sends a bounded evidence set plus a 2048-token JSON response. Keep
> `--ctx-size` at **16384** (the shipped unit default) so the full evidence
> prompt fits; if you must run a smaller window, lower
> `APEX_ANALYSIS_MAX_PROMPT_CHARS` accordingly and expect fewer observations per run.

### 5.6 Enable All Services

```bash
sudo systemctl daemon-reload
sudo systemctl enable nats minio apexintel-api apexintel-worker apexintel-llm
sudo systemctl start nats minio
sudo systemctl start apexintel-api
sudo systemctl start apexintel-worker
sudo systemctl start apexintel-llm
```

---

## 6. Nginx Configuration

The nginx config is maintained in the repository at `config/runtime/nginx-apexintel.conf`.

Create `/etc/nginx/sites-available/apexintel`:

```nginx
# ApexIntel – <PRODUCTION_DOMAIN>
# Rust/Axum serves HTML + API; nginx serves static assets (no more Next.js)

upstream apexintel_api {
    server 127.0.0.1:8080;
    keepalive 16;
}

server {
    server_name <PRODUCTION_DOMAIN> www.<PRODUCTION_DOMAIN>;

    # Security headers
    add_header X-Frame-Options "SAMEORIGIN" always;
    add_header X-Content-Type-Options "nosniff" always;
    add_header X-XSS-Protection "1; mode=block" always;
    add_header Referrer-Policy "strict-origin-when-cross-origin" always;

    # Logging
    access_log /var/log/nginx/apexintel_access.log;
    error_log  /var/log/nginx/apexintel_error.log;

    # Static assets — served by nginx directly with long cache
    location /static/ {
        alias /opt/apexintel/static/;
        expires 1y;
        add_header Cache-Control "public, immutable, max-age=31536000";
        access_log off;
    }

    # WebSocket support
    location /ws/ {
        proxy_pass http://apexintel_api;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_read_timeout 86400s;
    }

    # API routes (JSON endpoints)
    location /api/ {
        proxy_pass http://apexintel_api;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_set_header Connection "";
        proxy_connect_timeout 10s;
        proxy_read_timeout 120s;
        proxy_send_timeout 60s;
        client_max_body_size 64k;

        # CORS — hardcoded to known origin, never reflect $http_origin (CORS reflection vulnerability)
        add_header Access-Control-Allow-Origin "https://<PRODUCTION_DOMAIN>" always;
        add_header Access-Control-Allow-Methods "GET, POST, PUT, DELETE, OPTIONS" always;
        add_header Access-Control-Allow-Headers "Authorization, Content-Type, X-Api-Key" always;
        add_header Access-Control-Allow-Credentials "true" always;

        if ($request_method = OPTIONS) {
            return 204;
        }
    }

    # Everything else — Rust/Axum (HTML pages + login/logout)
    location / {
        proxy_pass http://apexintel_api;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_set_header Connection "";
        proxy_read_timeout 30s;
    }

    # Deny access to dotfiles (except .well-known for Let's Encrypt)
    location ~ /\.(?!well-known) {
        deny all;
    }

    listen [::]:443 ssl;
    listen 443 ssl;
    ssl_certificate /etc/letsencrypt/live/<PRODUCTION_DOMAIN>/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/<PRODUCTION_DOMAIN>/privkey.pem;
    include /etc/letsencrypt/options-ssl-nginx.conf;
    ssl_dhparam /etc/letsencrypt/ssl-dhparams.pem;
}

server {
    if ($host = <PRODUCTION_DOMAIN>) {
        return 301 https://$host$request_uri;
    }
    listen 80;
    listen [::]:80;
    server_name <PRODUCTION_DOMAIN> www.<PRODUCTION_DOMAIN>;
    return 404;
}
```

Enable site:
```bash
ln -sf /etc/nginx/sites-available/apexintel /etc/nginx/sites-enabled/apexintel
nginx -t && systemctl reload nginx
```

### 6.1 SSL Certificate

```bash
sudo certbot --nginx -d <PRODUCTION_DOMAIN> -d www.<PRODUCTION_DOMAIN>
```

---

## 7. LLM Model Setup

### 7.1 Model Weight Transfer

The trained Qwen3-30B-A3B model weights are on the vast.ai instance.

```bash
# On vast.ai: Quantize to GGUF Q4_K_M
cd /tmp && git clone https://github.com/ggerganov/llama.cpp.git
cd llama.cpp && cmake -B build -DGGML_CUDA=ON && cmake --build build -j$(nproc)

python3 convert_hf_to_gguf.py <TRAINING_OUTPUT_DIR>/merged_phase1/ \
  --outfile <TRAINING_OUTPUT_DIR>/Qwen3-30B-A3B-f16.gguf --outtype f16

./build/bin/llama-quantize \
  <TRAINING_OUTPUT_DIR>/Qwen3-30B-A3B-f16.gguf \
  <TRAINING_OUTPUT_DIR>/Qwen3-30B-A3B-Q4_K_M.gguf Q4_K_M
```

Transfer to Hetzner:
```bash
scp -i ~/.ssh/<DEPLOY_KEY> <TRAINING_OUTPUT_DIR>/Qwen3-30B-A3B-Q4_K_M.gguf \
  <DEPLOY_USER>@<PRODUCTION_HOST>:/opt/apexintel/model/
```

### 7.2 Verify Model

```bash
curl http://127.0.0.1:8081/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "Qwen3-30B-A3B-Q4_K_M",
    "messages": [{"role": "user", "content": "What is Starz Electronics?"}],
    "max_tokens": 100
  }'
```

---

## 8. Full Deployment Procedure (Quick Reference)

### First-Time Deployment

```bash
# 1. Server setup (sections 1.3–1.8)
ssh -i ~/.ssh/<DEPLOY_KEY> <DEPLOY_USER>@<PRODUCTION_HOST>

# 2. Database setup (section 2)
# 3. Environment config (section 4)
# 4. Install systemd services (section 5)
# 5. Cross-compile and upload:
# (on local machine)
cargo zigbuild --release --target aarch64-unknown-linux-gnu -p apex-api --features llm --locked
cargo zigbuild --release --target aarch64-unknown-linux-gnu -p apex-worker --features llm --locked
scp -i ~/.ssh/<DEPLOY_KEY> target/aarch64-unknown-linux-gnu/release/apex-api <DEPLOY_USER>@<PRODUCTION_HOST>:/opt/apexintel/bin/
scp -i ~/.ssh/<DEPLOY_KEY> target/aarch64-unknown-linux-gnu/release/apex-worker <DEPLOY_USER>@<PRODUCTION_HOST>:/opt/apexintel/bin/
scp -i ~/.ssh/<DEPLOY_KEY> -r crates/api/static/* <DEPLOY_USER>@<PRODUCTION_HOST>:/opt/apexintel/static/
ssh -i ~/.ssh/<DEPLOY_KEY> <DEPLOY_USER>@<PRODUCTION_HOST> "chown -R apexintel:apexintel /opt/apexintel && chmod 700 /opt/apexintel/bin/apex-api /opt/apexintel/bin/apex-worker"

# 6. Configure nginx + SSL (section 6)
# 7. Start services
systemctl start nats minio apexintel-api apexintel-worker apexintel-llm
```

### Routine Update (code changes)

```bash
# On local machine:
cd <REPO_ROOT>

# 1. Build
cargo zigbuild --release --target aarch64-unknown-linux-gnu -p apex-api --features llm --locked
cargo zigbuild --release --target aarch64-unknown-linux-gnu -p apex-worker --features llm --locked

# 2. Upload
scp -i ~/.ssh/<DEPLOY_KEY> \
  target/aarch64-unknown-linux-gnu/release/apex-api \
  <DEPLOY_USER>@<PRODUCTION_HOST>:/tmp/apex-api-new
scp -i ~/.ssh/<DEPLOY_KEY> \
  target/aarch64-unknown-linux-gnu/release/apex-worker \
  <DEPLOY_USER>@<PRODUCTION_HOST>:/tmp/apex-worker-new

# 3. Preflight the schema (mandatory for the fail-closed boot)
#    With APEX_SKIP_MIGRATIONS=true (production) the new binaries refuse to
#    start unless the applied migration history matches their embedded
#    migrations (latest version + every checksum). If the DB is behind,
#    apply the pending migrations (rehearsed on a restored backup) first.
ssh -i ~/.ssh/<DEPLOY_KEY> root@<PRODUCTION_HOST> \
  'psql "$(grep ^DATABASE_URL= /opt/apexintel/config/.env | cut -d= -f2-)" \
     -Atc "SELECT max(version) FROM _sqlx_migrations WHERE success"'

# 4. Install & restart
ssh -i ~/.ssh/<DEPLOY_KEY> root@<PRODUCTION_HOST> << 'EOF'
systemctl stop apexintel-api apexintel-worker
cp /tmp/apex-api-new /opt/apexintel/bin/apex-api
cp /tmp/apex-worker-new /opt/apexintel/bin/apex-worker
chmod 700 /opt/apexintel/bin/apex-api /opt/apexintel/bin/apex-worker
chown apexintel:apexintel /opt/apexintel/bin/apex-api /opt/apexintel/bin/apex-worker
systemctl start apexintel-api apexintel-worker
curl -sf http://127.0.0.1:8080/api/health | jq
EOF

# 5. Record deployment provenance (git SHA + build timestamp + CI pipeline +
#    test results + migration test result + artifact digest + deploy time)
#    against the exact artifact uploaded in step 2. The digest is computed
#    locally from the same bytes that were copied to the server.
APEX_GIT_SHA="$(git rev-parse HEAD)"
APEX_CI_PIPELINE_ID="${CI_PIPELINE_NUMBER:-local}"
APEX_BUILD_TIMESTAMP="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
APEX_ARTIFACT_DIGEST="sha256:$(shasum -a 256 target/aarch64-unknown-linux-gnu/release/apex-api | awk '{print $1}')"
APEX_DEPLOYED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
# 5a. Release evidence bundle for this exact SHA. CI writes this in the
#     release-evidence step; locally it fails unless every executed gate
#     passed, and any gate owned by another run can be recorded with
#     --record <gate>=passed. The bundle is written to
#     release-evidence/<sha>/release-evidence.{json,txt}.
scripts/ci/release_evidence.sh \
  --sha "${APEX_GIT_SHA}" \
  --gate exact-sha \
  --gate rustfmt \
  --record clippy-default=passed \
  --record clippy-all-features=passed \
  --record unit-tests=passed \
  --record all-features-tests=passed \
  --record pg-canonical=passed \
  --record migration-bootstrap=passed \
  --record browser-integration=passed \
  --record ui-journey=passed \
  --record tailwind-assets=passed \
  --record wasm-shared=passed
scripts/ops/record_deployment.sh \
  --git-sha "${APEX_GIT_SHA}" \
  --build-timestamp "${APEX_BUILD_TIMESTAMP}" \
  --pipeline-id "${APEX_CI_PIPELINE_ID}" \
  --tests passed \
  --migrations passed \
  --artifact-digest "${APEX_ARTIFACT_DIGEST}" \
  --deployed-at "${APEX_DEPLOYED_AT}" \
  --ledger deployments.jsonl

# 6. Give the running API that identity, then verify it. systemd reads
#    EnvironmentFile at process start, so the restart is required; appending
#    is safe because the later value wins.
ssh -i ~/.ssh/<DEPLOY_KEY> root@<PRODUCTION_HOST> "cat >> /opt/apexintel/config/.env <<EOF
APEX_GIT_SHA=${APEX_GIT_SHA}
APEX_BUILD_TIMESTAMP=${APEX_BUILD_TIMESTAMP}
APEX_CI_PIPELINE_ID=${APEX_CI_PIPELINE_ID}
APEX_ARTIFACT_DIGEST=${APEX_ARTIFACT_DIGEST}
APEX_DEPLOYED_AT=${APEX_DEPLOYED_AT}
EOF
systemctl restart apexintel-api"
ssh -i ~/.ssh/<DEPLOY_KEY> root@<PRODUCTION_HOST> \
  'curl -sf http://127.0.0.1:8080/api/version | jq'
# Expected: git_sha, build_timestamp and artifact_digest match the values
# echoed by the recorder and the release-evidence bundle, and configured is
# true.
```

### Updating Static Assets Only

```bash
scp -i ~/.ssh/<DEPLOY_KEY> -r crates/api/static/* <DEPLOY_USER>@<PRODUCTION_HOST>:/opt/apexintel/static/
ssh -i ~/.ssh/<DEPLOY_KEY> <DEPLOY_USER>@<PRODUCTION_HOST> "chown -R apexintel:apexintel /opt/apexintel/static"
# No service restart needed — nginx serves static files directly
```

---

## 9. Verification

### 9.1 Service Health

```bash
systemctl status apexintel-api apexintel-worker apexintel-llm nats minio postgresql redis
```

### 9.2 API Health Check

```bash
# Detailed capability matrix
curl -s https://<PRODUCTION_DOMAIN>/api/health | jq
# Expected: {"status":"Healthy","version":"0.1.0","checks":[...]}

# Profile-aware readiness probe (503 when a required capability is not proven)
curl -s -o /dev/null -w '%{http_code}\n' https://<PRODUCTION_DOMAIN>/api/health/ready
# APEX_PROFILE=core (default) requires database, schema lineage, worker
# heartbeat, embeddings and search index. APEX_PROFILE=full composes the
# product surfaces and additionally requires the LLM endpoint, NATS, browser
# rendering, crawl freshness, minimum operational source coverage, alert-rule
# engine state, outbox publisher backlog, durable notification-delivery health
# (retry processor freshness, backlog/dead-letter policy, stuck leases, recent
# success ratio) and critical scheduled-job freshness; it refuses to start
# without a `--features llm` build. The response publishes the exact required
# set and the resolved thresholds as policy, plus the measured
# `schema_lineage` proof: the embedded (expected) migration head, the applied
# migration head, whether the applied history checksum-verifies, and a
# mismatch verdict.

# Product surface probes (same measured capabilities, scoped per surface)
curl -s https://<PRODUCTION_DOMAIN>/process/live       # 200 while the API serves
curl -s https://<PRODUCTION_DOMAIN>/process/ready      # database, worker, scheduled jobs
curl -s https://<PRODUCTION_DOMAIN>/data/healthy       # freshness, sources, index, browser
curl -s https://<PRODUCTION_DOMAIN>/intelligence/healthy  # LLM, embeddings, alert engine
curl -s https://<PRODUCTION_DOMAIN>/delivery/healthy   # NATS, outbox publisher, notification delivery
# Expected: {"surface":"...","status":"ok","checks":[...]}
# Policy thresholds are configurable without a rebuild (defaults in parens):
#   APEX_LLM_PROBE_TTL_SECS (60)         APEX_LLM_PROBE_TIMEOUT_SECS (5)
#   APEX_BROWSER_PROBE_TTL_SECS (300)    APEX_BROWSER_PROBE_TIMEOUT_SECS (30)
#   APEX_EMBEDDING_CANARY_TTL_SECS (300) APEX_EMBEDDING_CANARY_TIMEOUT_SECS (10)
#   APEX_SEARCH_INDEX_MAX_LAG_SECS (3600)
#   APEX_CRAWL_FRESHNESS_MAX_AGE_SECS (21600)
#   APEX_COVERAGE_<FAMILY>_REQUIRED/_MIN/_FRESHNESS_SECS/_FETCH_SUCCESS_PCT/
#     _PARSER_SUCCESS_PCT/_MIN_DOMAINS (per-family coverage matrix)
#   APEX_COVERAGE_PRIORITY_COMPANY_PCT (50)
#   APEX_COVERAGE_PRIORITY_COMPANY_WINDOW_SECS (2592000)
#   APEX_ALERT_ENGINE_MAX_AGE_SECS (900)
#   APEX_OUTBOX_MAX_PENDING (100)        APEX_OUTBOX_MAX_OLDEST_PENDING_SECS (300)
#   APEX_NOTIFICATION_DELIVERY_MAX_PROCESSOR_AGE_SECS (900)
#   APEX_NOTIFICATION_DELIVERY_MAX_OVERDUE (250)
#   APEX_NOTIFICATION_DELIVERY_MAX_OVERDUE_AGE_SECS (600)
#   APEX_NOTIFICATION_DELIVERY_MAX_DEAD_LETTERED (25)
#   APEX_NOTIFICATION_DELIVERY_MAX_DEAD_LETTER_RATE (10)
#   APEX_NOTIFICATION_DELIVERY_MAX_STUCK_LEASES (0)
#   APEX_NOTIFICATION_DELIVERY_SUCCESS_WINDOW_SECS (3600)
#   APEX_NOTIFICATION_DELIVERY_MIN_RECENT_ATTEMPTS (5)
#   APEX_NOTIFICATION_DELIVERY_MIN_SUCCESS_PERCENT (90)
#   APEX_CRITICAL_JOBS (crawl_cycle,triage_processing)
#   APEX_CRITICAL_JOB_MAX_AGE_SECS (7200)
# Running deployment provenance (git SHA, build timestamp, CI pipeline,
# artifact digest, deploy time)
curl -s https://<PRODUCTION_DOMAIN>/api/version | jq
# Expected: {"service":"apex-api","git_sha":"<sha>",
#            "build_timestamp":"<rfc3339-or-epoch>","ci_pipeline_id":"<n>",
#            "artifact_digest":"sha256:<hex>","deployed_at":"<rfc3339>",
#            "configured":true}
# Match git_sha + artifact_digest against
# release-evidence/<sha>/release-evidence.json to prove the deployed binary
# came from the evidenced build.
# The `schema` capability in /api/health/capabilities reports the same applied
# migration head / checksum evidence as `schema_lineage` in
# /api/health/ready, and degrades when the applied head is not the embedded
# head.
```

Worker containers run `apex-worker healthcheck` as their Docker healthcheck:
it fails when the database is unreachable, no worker heartbeat exists, or the
newest `service_heartbeats` row is older than 120s (a stalled scheduler stops
writing heartbeats).

### 9.3 Web UI

```bash
curl -sI https://<PRODUCTION_DOMAIN>/login
# Expected: HTTP/2 200, content-type: text/html
```

### 9.4 Static Assets

```bash
curl -sI https://<PRODUCTION_DOMAIN>/static/css/tailwind.css
# Expected: HTTP/2 200, Cache-Control: public, immutable, max-age=31536000
```

### 9.5 LLM

```bash
curl -s http://127.0.0.1:8081/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model":"Qwen3-30B-A3B-Q4_K_M","messages":[{"role":"user","content":"test"}],"max_tokens":10}' | jq
```

---

## 10. Common Operations

```bash
# ─── SSH Access ─────────────────────────────────────────────
ssh -i ~/.ssh/<DEPLOY_KEY> <DEPLOY_USER>@<PRODUCTION_HOST>

# ─── Service Management ────────────────────────────────────
sudo systemctl restart apexintel-api
sudo systemctl restart apexintel-worker
sudo systemctl restart apexintel-llm
sudo systemctl status apexintel-api apexintel-worker apexintel-llm

# ─── Logs ──────────────────────────────────────────────────
tail -f /opt/apexintel/logs/api.log
tail -f /opt/apexintel/logs/worker.log
tail -f /opt/apexintel/logs/llm.log
tail -f /var/log/nginx/apexintel_error.log
tail -f /var/log/nginx/apexintel_access.log

# ─── Database ─────────────────────────────────────────────
sudo -u postgres psql apexintel
sudo -u postgres psql apexintel -c "SELECT COUNT(*) FROM companies;"

# ─── Backup ───────────────────────────────────────────────
sudo -u postgres pg_dump apexintel | gzip > /opt/apexintel/backups/db_$(date +%Y%m%d).sql.gz
```

---

## 11. Security Hardening

### 11.1 Firewall (UFW)

```bash
sudo ufw allow 22/tcp
sudo ufw allow 80/tcp
sudo ufw allow 443/tcp
sudo ufw default deny incoming
sudo ufw default allow outgoing
sudo ufw enable
```

### 11.2 Bind Internal Services to Localhost

All internal services (PostgreSQL, Redis, NATS, MinIO, llama-server) listen on `127.0.0.1` only.

### 11.3 File Permissions

```bash
chmod 600 /opt/apexintel/config/.env
chmod 700 /opt/apexintel/bin/apex-api /opt/apexintel/bin/apex-worker
chown -R apexintel:apexintel /opt/apexintel
chmod o+x /opt/apexintel  # allow nginx to traverse to static/
```

---

## 12. Monitoring

### 12.1 Health Check Script

Create `/opt/apexintel/bin/health-check.sh`:

```bash
#!/bin/bash
echo "=== ApexIntel Health Check $(date) ==="

curl -sf http://127.0.0.1:8080/api/health >/dev/null && echo "✅ API + Web UI" || echo "❌ API + Web UI"
redis-cli ping | grep -q PONG && echo "✅ Redis" || echo "❌ Redis"
sudo -u postgres psql apexintel -c "SELECT 1" >/dev/null 2>&1 && echo "✅ PostgreSQL" || echo "❌ PostgreSQL"
curl -sf http://127.0.0.1:8081/health >/dev/null && echo "✅ LLM Server" || echo "❌ LLM Server"

echo "Disk: $(df -h /opt/apexintel | tail -1 | awk '{print $5}')"
echo "Memory: $(free -h | grep Mem | awk '{print $3 "/" $2}')"
```

### 12.2 Cron Jobs

```bash
0 */6 * * * /opt/apexintel/bin/health-check.sh >> /opt/apexintel/logs/health.log 2>&1
0 3 * * * sudo -u postgres pg_dump apexintel | gzip > /opt/apexintel/backups/db_$(date +\%Y\%m\%d).sql.gz
7 3 * * * find /opt/apexintel/backups -name "db_*.sql.gz" -mtime +30 -delete
```

---

## Appendix A – Connection Details

| Service | Address | Notes |
|---------|---------|-------|
| **SSH** | `ssh -i ~/.ssh/<DEPLOY_KEY> <DEPLOY_USER>@<PRODUCTION_HOST>` | |
| **Vast.ai** | `ssh -i ~/.ssh/<DEPLOY_KEY> <DEPLOY_USER>@<MODEL_HOST>` | Model weights source |
| **HTTPS** | `https://<PRODUCTION_DOMAIN>` | Production URL |
| **API + Web UI** | `http://127.0.0.1:8080` (internal) | `apex-api` Axum server |
| **LLM** | `http://127.0.0.1:8081` (internal) | llama-server |
| **PostgreSQL** | `127.0.0.1:5432` | DB: `apexintel` |
| **Redis** | `127.0.0.1:6379` | |
| **NATS** | `127.0.0.1:4222` | |
| **MinIO** | `127.0.0.1:9000` (API), `127.0.0.1:9001` (console) | |

## Appendix B – Architecture Change Log

### March 2026 — Server-rendered UI Migration (multi-service)

The Next.js frontend was fully replaced by Askama (Jinja2-like) templates + HTMX.
Templates are compiled into the `apex-api` binary; static assets are **not**
compiled in and ship alongside the binaries:

- **Removed**: Node.js, Next.js, `apexintel-frontend` systemd service, `/opt/apexintel/frontend/`
- **Added**: 45 HTML templates in `crates/api/templates/`, compiled into `apex-api`
- **Added**: 11 static assets in `crates/api/static/`, deployed to `/opt/apexintel/static/` and served by nginx from disk
- **Added**: Cookie-based session auth in the API binary (login/logout endpoints)
- **Changed**: Nginx now routes all traffic to one upstream (port 8080)
- **Changed**: Static assets served directly by nginx from `/opt/apexintel/static/`
- **Result**: ~764 MB freed on server; the API binary serves the UI, but production
  still runs three services: `apexintel-api`, `apexintel-worker`, `apexintel-llm`

## Appendix C – Deployment Status

- [x] Deployment guide updated for the server-rendered UI (multi-service: api/worker/llm)
- [x] Hetzner firewall opened (ports 22, 80, 443)
- [x] Server setup (packages, users, directories)
- [x] PostgreSQL + Redis configured
- [x] NATS installed
- [x] MinIO installed
- [x] Rust binaries deployed (apex-api + apex-worker, ARM64)
- [x] Static assets deployed (/opt/apexintel/static/)
- [x] Environment configured (.env with auth credentials)
- [x] Systemd services updated (ReadOnlyPaths for static)
- [x] Nginx updated (apex-api upstream, static alias)
- [x] SSL certificate active
- [x] Old Next.js frontend removed from server
- [x] Old frontend systemd service removed
- [x] Full system verified (Mar 4, 2026 — API healthy, 45 endpoints, static serving OK)

## Appendix D – Migration Lineage Reconciliation (Aug 27, 2026; historical)

**Historical record.** The one-time reconciliation below describes the schema
state at that date. The secondary `crates/store/migrations/` lineage referenced
in this appendix has since been deleted: `migrations/` is the only lineage
(enforced by `scripts/ci/check_single_migrations_folder.sh`), and the pending
migration head has moved past version `043`.

Production's `_sqlx_migrations` was built from a **consolidated** set
(`crates/store/migrations/0001–0014` + `20260701_sales_activation_layer`)
that no longer matches the repository's embedded migration directory
(`migrations/` → versions `000`–`043`, renumbered for sqlx uniqueness in the
B353 batch). Deploying any binary built from the repo would have aborted at
boot with checksum/version-mismatch errors.

Reconciliation performed (one-time, 2026-08-27, after a full `pg_dump` backup):

1. **Schema diff** — fresh reference DB (bootstrapped from the new binary)
   vs production: shared tables were column-complete; 17 tables + 4 views
   existed only in the reference and were created on production from
   `pg_dump`-extracted definitions (`alert_rules`, `social_signals`,
   `llm_response_cache`, `insight_generation_log`, supplier/supply-chain
   set, `strategic_predictions`, `weekly_memo_recipients`, …).
2. **Lineage normalization** — old `_sqlx_migrations` rows preserved in
   `_sqlx_migrations_lineage_backup_20260827`; the table rewritten to the
   44 repository migrations (versions 0–43) with their true SHA-384
   checksums, all marked applied. Future migrations append cleanly.
3. **Dropped hand-patched constraint** — `warnings_recipe_code_fkey`
   (production-only; the code treats `warnings.recipe_code` as a free-form
   provenance tag, see B355).

**Rule going forward**: never hand-edit production schema outside the repo's
`migrations/` directory — the API validates checksums at boot and a
divergent lineage blocks startup.

### D.1 Production tuning knobs added

| Env var | Default | Purpose |
|---------|---------|---------|
| `WORKER_MAX_CONCURRENT_JOBS` | 4 | Scheduler-wide concurrency permit count |
| `LLM_INSIGHT_MAX_COMPANIES` | 10 | Companies per insight-generation run (CPU LLM budget) |
| `LLM_TIMEOUT_SECS` | 180 (code default); production sets 600 | Per-call LLM timeout; the 180 default is too small for 30B CPU inference of 4k-token prompts, so production `.env` sets 600 |
| `LOOKALIKE_MAX_CHECKS_PER_DOMAIN` | 15 | DNS verification budget per domain in the lookalike scan |
| `POI_LLM_ENRICH_PER_RUN` | 40 | Bounded POI enrichment per nightly run |
| `API_TRUST_PROXY` | unset | Set `1` only behind a proxy that overwrites `X-Forwarded-For` |

### D.2 Verified at deploy (2026-08-27)

All seven services active; `/api/health` Healthy over HTTPS; all 26
authenticated pages HTTP 200 (session cookie verified server-side); all 7
static assets 200 via nginx; `/metrics` 401 without a key / 200 with;
session-authenticated `/api/*` 200; admin routes 403 for Analyst sessions;
`/api/trends` 200 (previously permanent 500); search partial renders all
five facets; worker: 88 runs/24 h with 0 failures, DB leases claiming,
crawl cycle + enrichment writing observations; LLM pipeline verified
end-to-end (loader 0→14 companies, llama-server evaluating prompts,
quality gates accepting/rejecting generated narratives); zero ERROR lines
in both units post-deploy.
