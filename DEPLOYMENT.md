# ApexIntel – Production Deployment Guide

**Audience**: DevOps, System Administrator  
**Revision**: July 2025  
**Status**: 🚀 Initial deployment  
**Domain**: https://starzerp.fi

---

## 0. Target Production Environment

| Item | Value |
|------|-------|
| **Domain** | [https://starzerp.fi](https://starzerp.fi) |
| **VPS Provider** | Hetzner |
| **VPS IPv4** | `77.42.65.89` |
| **OS** | Ubuntu 24.04 LTS (expected) |
| **SSH User** | `root` (initial) |
| **Colocated with** | CRM-v2 (Starz Morocco CRM) — **completely separate** |

### 0.1 SSH Key Setup

| Item | Value |
|------|-------|
| **Private key** | `~/.ssh/hetzner-db-mac` (local machine) |
| **Public key** | `~/.ssh/hetzner-db-mac.pub` |

**Connect to server:**
```bash
ssh -i ~/.ssh/hetzner-db-mac root@77.42.65.89
```

---

### 0.2 Application Stack

| Component | Version | Purpose |
|-----------|---------|---------|
| **Rust** | 1.80+ | Backend API + Worker (compiled binaries) |
| **Node.js** | 20.x LTS | Frontend build (Next.js SSR) |
| **PostgreSQL** | 16+ | Primary database |
| **Redis** | 7+ | Cache + session |
| **NATS** | 2.10+ | Message queue (inter-crate async messaging) |
| **MinIO** | Latest | S3-compatible object storage |
| **llama-server** | Latest (llama.cpp) | LLM inference for trained Qwen3-30B-A3B |
| **Nginx** | 1.24+ | Reverse proxy + TLS termination |
| **Certbot** | Latest | Let's Encrypt SSL auto-renewal |

### 0.3 Separation from CRM-v2

The CRM-v2 system may coexist on this server. **Everything must be isolated:**

| Resource | CRM-v2 | ApexIntel |
|----------|--------|-----------|
| **App directory** | `/var/www/crm-starz-morocco/` | `/opt/apexintel/` |
| **Nginx vhost** | `/etc/nginx/sites-available/starzcrm` | `/etc/nginx/sites-available/apexintel` |
| **Database** | MySQL `starz_crm` | PostgreSQL `apexintel` |
| **Systemd services** | `starz-messenger` | `apexintel-api`, `apexintel-worker`, `apexintel-frontend`, `apexintel-llm` |
| **Ports (internal)** | PHP-FPM socket | API: 8080, Frontend: 3000, LLM: 8081, NATS: 4222, MinIO: 9000, Redis: 6379, PG: 5432 |
| **Domain** | starzcrm.com | starzerp.fi |
| **User** | www-data | apexintel |

---

## 1. Server Setup (Fresh)

### 1.1 System Requirements

**Minimum**: 8 vCPU, 32 GB RAM, 200 GB SSD, 1 Gbps NIC  
**Recommended for LLM**: 16+ vCPU, 64+ GB RAM (Qwen3-30B-A3B Q4_K_M needs ~17 GB RAM)  
**Network**: Ports 22, 80, 443 open inbound; all outbound open

### 1.2 Hetzner Cloud Firewall

**CRITICAL**: Open the following inbound rules in the Hetzner Cloud Console firewall:

| Protocol | Port | Source | Purpose |
|----------|------|--------|---------|
| TCP | 22 | Any (or your IP) | SSH access |
| TCP | 80 | Any | HTTP (Certbot + redirect) |
| TCP | 443 | Any | HTTPS (production traffic) |

### 1.3 Create System User

```bash
useradd -r -m -d /opt/apexintel -s /bin/bash apexintel
mkdir -p /opt/apexintel/{bin,data,logs,model,config}
chown -R apexintel:apexintel /opt/apexintel
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
  cmake gcc g++
```

### 1.5 Install Rust

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source $HOME/.cargo/env
rustup default stable
```

### 1.6 Install Node.js 20.x

```bash
curl -fsSL https://deb.nodesource.com/setup_20.x | sudo -E bash -
sudo apt install -y nodejs
```

### 1.7 Install NATS Server

```bash
curl -sf https://binaries.nats.dev/nats-io/nats-server/v2@latest | sh
sudo mv nats-server /usr/local/bin/
```

### 1.8 Install MinIO

```bash
wget https://dl.min.io/server/minio/release/linux-amd64/minio
chmod +x minio
sudo mv minio /usr/local/bin/
mkdir -p /opt/apexintel/data/minio
```

### 1.9 Install llama.cpp (llama-server)

```bash
cd /tmp
git clone https://github.com/ggerganov/llama.cpp.git
cd llama.cpp
cmake -B build -DGGML_BLAS=ON -DGGML_BLAS_VENDOR=OpenBLAS
cmake --build build --config Release -j$(nproc)
sudo cp build/bin/llama-server /usr/local/bin/
```

> **Note**: For CPU-only inference, OpenBLAS provides adequate performance.
> If the server has a GPU, use `-DGGML_CUDA=ON` instead.

---

## 2. Database Setup

### 2.1 PostgreSQL

**⚠️ SECURITY: Generate a strong password before running these commands.**

```bash
# Generate a secure password (copy this output):
openssl rand -base64 32

# Create the database with YOUR generated password:
sudo -u postgres psql <<'SQL'
CREATE USER apexintel WITH PASSWORD '<YOUR_GENERATED_PASSWORD>';
CREATE DATABASE apexintel OWNER apexintel;
GRANT ALL PRIVILEGES ON DATABASE apexintel TO apexintel;
\c apexintel
CREATE EXTENSION IF NOT EXISTS "uuid-ossp";
CREATE EXTENSION IF NOT EXISTS "pg_trgm";
SQL
```

**Connection string template:**
```
postgresql://apexintel:<YOUR_GENERATED_PASSWORD>@127.0.0.1:5432/apexintel
```

**Store this password securely** — you'll need it for the `.env` file in Section 4.

### 2.2 Redis

Default config is fine. Verify:
```bash
sudo systemctl enable redis-server
sudo systemctl start redis-server
redis-cli ping  # should return PONG
```

---

## 3. Application Deployment

### 3.1 File System Layout

```
/opt/apexintel/                          ← Application root (owner: apexintel)
├── bin/
│   ├── apex-api                         ← Rust API server binary
│   └── apex-worker                      ← Rust worker binary
├── config/
│   └── .env                             ← Production environment (secrets)
├── data/
│   ├── search/                          ← Tantivy full-text index
│   └── minio/                           ← MinIO object storage data
├── frontend/                            ← Next.js production build
│   ├── .next/                           ← Compiled Next.js output
│   ├── node_modules/                    ← Dependencies
│   ├── package.json
│   └── ...
├── model/                               ← LLM model weights (GGUF)
│   └── Qwen3-30B-A3B-Q4_K_M.gguf       ← Quantized model (~17 GB)
└── logs/
    ├── api.log
    ├── worker.log
    └── llm.log
```

### 3.2 Build on Server

#### Option A: Build on server (recommended for first deploy)

```bash
# Clone repository
cd /opt/apexintel
sudo -u apexintel git clone <repository-url> /opt/apexintel/src
cd /opt/apexintel/src

# Build Rust binaries (release mode, with LLM feature)
cargo build --release -p apex-api --features llm
cargo build --release -p apex-worker --features llm

# Copy binaries
cp target/release/apex-api /opt/apexintel/bin/
cp target/release/apex-worker /opt/apexintel/bin/
chown apexintel:apexintel /opt/apexintel/bin/*

# Build frontend
cd frontend
npm ci
NEXT_PUBLIC_API_BASE_URL=https://starzerp.fi npm run build
cp -r . /opt/apexintel/frontend/
chown -R apexintel:apexintel /opt/apexintel/frontend/
```

#### Option B: Upload tarball from local machine

```bash
# On local machine:
cd ~/IdeaProjects/ApexIntel

tar czf /tmp/apexintel-deploy.tar.gz \
  --exclude='./target' \
  --exclude='./frontend/node_modules' \
  --exclude='./frontend/.next' \
  --exclude='./.git' \
  --exclude='./training' \
  --exclude='./training_data' \
  --exclude='./.env' \
  --exclude='./.env.local' \
  .

# Upload
scp -i ~/.ssh/hetzner-db-mac /tmp/apexintel-deploy.tar.gz root@77.42.65.89:/tmp/

# On server:
sudo -u apexintel mkdir -p /opt/apexintel/src
cd /opt/apexintel/src
sudo -u apexintel tar xzf /tmp/apexintel-deploy.tar.gz

# Build as above
```

### 3.3 Run Database Migrations

Migrations are embedded in the `apex-store` crate and run automatically when the API starts (`store.run_migrations().await`). They can also be triggered manually:

```bash
# The API binary runs migrations at startup automatically.
# First start will create all tables (companies, sites, warnings, insights, etc.)
```

---

## 4. Environment Configuration

Create `/opt/apexintel/config/.env`:

```dotenv
# ══════════════════════════════════════════════════════════════════════════════
# ApexIntel Production Environment Configuration
# ══════════════════════════════════════════════════════════════════════════════
# ⚠️ SECURITY: This file contains sensitive credentials. Ensure:
#   1. File permissions are 600 (chmod 600 .env)
#   2. File is owned by apexintel user
#   3. Never commit this file to version control
#   4. Generate ALL secrets fresh for production (see commands below)
# ══════════════════════════════════════════════════════════════════════════════

# ─── Required (MUST CONFIGURE) ───────────────────────────────────────────────
# Generate database password: openssl rand -base64 32
DATABASE_URL=postgresql://apexintel:<DB_PASSWORD>@127.0.0.1:5432/apexintel

# ─── Service URLs (local defaults) ───────────────────────────────────────────
REDIS_URL=redis://127.0.0.1:6379
NATS_URL=nats://127.0.0.1:4222
MINIO_URL=http://127.0.0.1:9000
MINIO_BUCKET=apexintel

# ─── LLM (local llama-server) ────────────────────────────────────────────────
LLM_BASE_URL=http://127.0.0.1:8081
LLM_MODEL=Qwen3-30B-A3B-Q4_K_M

# ─── API Server ──────────────────────────────────────────────────────────────
HOST=127.0.0.1
PORT=8080
CORS_ORIGIN=https://starzerp.fi
API_LOG_LEVEL=info
SEARCH_INDEX_PATH=/opt/apexintel/data/search

# ─── API Authentication ──────────────────────────────────────────────────────
# Generate API key: openssl rand -hex 32 | sed 's/^/sk-apex-/'
# Format: <raw_key>,<display_name>,<role>
# Roles: admin, analyst, viewer, service
API_KEY_1=<GENERATE_API_KEY>,Production Admin,admin

# ─── Crawl & Scheduling ──────────────────────────────────────────────────────
CRAWL_INTERVAL_SECS=21600
NIGHTLY_HOUR_UTC=2
WEEKLY_DAY=0
DEFAULT_RPS=0.2
PROXY_POOL_SIZE=50
ENABLE_PROXY_ROTATION=false
ENABLE_HEADLESS_BROWSER=false

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

### 4.1 Frontend Environment Requirements

The frontend requires these environment variables (set in systemd service or `.env.local`):

| Variable | Required | Description | Generation Command |
|----------|----------|-------------|-------------------|
| `APEX_ADMIN_USERNAME` | ✅ | Admin login username | Choose a username |
| `APEX_ADMIN_PASSWORD_HASH` | ✅ | SHA-256 hash of password | `echo -n 'password' \| sha256sum \| cut -d' ' -f1` |
| `SESSION_SECRET` | ✅ | 64-char hex for HMAC signing | `openssl rand -hex 32` |
| `APEX_API_KEY` | ✅ | Backend API key (raw, no prefix) | Must match `API_KEY_1` raw key |
| `APEX_API_BASE_URL` | ✅ | Internal backend URL | `http://127.0.0.1:8080` |
| `NEXT_PUBLIC_API_BASE_URL` | ✅ | Public base URL | `https://starzerp.fi` |

### 4.2 Startup Validation Behavior

**Production mode (`NODE_ENV=production`)**:
- Missing/invalid auth credentials → Server exits with code 1
- Missing `APEX_API_KEY` → Server exits with code 1
- Startup fails fast to prevent running with insecure defaults

**Development mode**:
- Missing credentials → Console warnings only
- Server starts but authentication may be misconfigured
- Useful for local development with mock auth

**Validation log examples**:
```
✅ Auth config valid             # All credentials configured
⚠️ Auth not configured           # Dev mode warning
❌ FATAL: Auth config invalid    # Production failure
```

### 4.3 Backend Environment Requirements Summary

| Variable | Required | Description |
|----------|----------|-------------|
| `DATABASE_URL` | ✅ | PostgreSQL connection string |
| `API_KEY_1` | ✅ | API key in format: `<raw_key>,<name>,<role>` |
| `REDIS_URL` | ✅ | Redis connection URL |
| `NATS_URL` | ✅ | NATS messaging URL |
| `LLM_BASE_URL` | ✅ | Local LLM inference endpoint |
| `CORS_ORIGIN` | ✅ | Allowed CORS origin |

---

## 5. Systemd Services

### 5.1 NATS Server

Create `/etc/systemd/system/nats.service`:

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

Create `/etc/systemd/system/minio.service`:

```ini
[Unit]
Description=MinIO Object Storage
After=network.target

[Service]
Type=simple
User=apexintel
Environment="MINIO_ROOT_USER=apexintel"
# ⚠️ SECURITY: Generate a unique password: openssl rand -base64 24
Environment="MINIO_ROOT_PASSWORD=<GENERATE_MINIO_PASSWORD>"
ExecStart=/usr/local/bin/minio server /opt/apexintel/data/minio --address 127.0.0.1:9000 --console-address 127.0.0.1:9001
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
```

### 5.3 ApexIntel API Server

Create `/etc/systemd/system/apexintel-api.service`:

```ini
[Unit]
Description=ApexIntel API Server (Rust/Axum)
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

# Logging
StandardOutput=append:/opt/apexintel/logs/api.log
StandardError=append:/opt/apexintel/logs/api.log

[Install]
WantedBy=multi-user.target
```

### 5.4 ApexIntel Worker

Create `/etc/systemd/system/apexintel-worker.service`:

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

StandardOutput=append:/opt/apexintel/logs/worker.log
StandardError=append:/opt/apexintel/logs/worker.log

[Install]
WantedBy=multi-user.target
```

### 5.5 ApexIntel Frontend (Next.js)

Create `/etc/systemd/system/apexintel-frontend.service`:

```ini
[Unit]
Description=ApexIntel Frontend (Next.js SSR)
After=network.target apexintel-api.service

[Service]
Type=simple
User=apexintel
WorkingDirectory=/opt/apexintel/frontend
Environment="NODE_ENV=production"
Environment="PORT=3000"
Environment="NEXT_PUBLIC_API_BASE_URL=https://starzerp.fi"
Environment="APEX_API_BASE_URL=http://127.0.0.1:8080"
# ⚠️ Auth credentials - generate all values before deployment:
#   Username: your admin username
#   Password hash: echo -n 'yourpassword' | sha256sum | cut -d' ' -f1
#   Session secret: openssl rand -hex 32
#   API key: must match API_KEY_1 in backend .env (the raw key part)
Environment="APEX_ADMIN_USERNAME=<YOUR_ADMIN_USERNAME>"
Environment="APEX_ADMIN_PASSWORD_HASH=<SHA256_OF_YOUR_PASSWORD>"
Environment="SESSION_SECRET=<GENERATE_64_CHAR_HEX_SECRET>"
Environment="APEX_API_KEY=<SAME_AS_API_KEY_1_RAW_KEY>"
ExecStart=/usr/bin/node node_modules/.bin/next start -p 3000
Restart=on-failure
RestartSec=5

StandardOutput=append:/opt/apexintel/logs/frontend.log
StandardError=append:/opt/apexintel/logs/frontend.log

[Install]
WantedBy=multi-user.target
```

### 5.6 LLM Inference Server (llama-server)

Create `/etc/systemd/system/apexintel-llm.service`:

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

StandardOutput=append:/opt/apexintel/logs/llm.log
StandardError=append:/opt/apexintel/logs/llm.log

[Install]
WantedBy=multi-user.target
```

### 5.7 Enable All Services

```bash
sudo systemctl daemon-reload
sudo systemctl enable nats minio apexintel-api apexintel-worker apexintel-frontend apexintel-llm
sudo systemctl start nats minio
sudo systemctl start apexintel-api
sudo systemctl start apexintel-worker
sudo systemctl start apexintel-frontend
sudo systemctl start apexintel-llm
```

---

## 6. Nginx Configuration

Create `/etc/nginx/sites-available/apexintel`:

```nginx
# ApexIntel — HTTPS (port 443)
server {
    server_name starzerp.fi www.starzerp.fi;

    # Frontend (Next.js SSR)
    location / {
        proxy_pass http://127.0.0.1:3000;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection 'upgrade';
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_cache_bypass $http_upgrade;
        proxy_read_timeout 300s;
    }

    # API (Rust/Axum) — Next.js rewrites /api/* → backend
    # This catches direct API calls that bypass the frontend
    location /api/ {
        proxy_pass http://127.0.0.1:8080;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_read_timeout 300s;
        proxy_send_timeout 300s;
        client_max_body_size 64k;
    }

    # WebSocket (live updates)
    location /ws/ {
        proxy_pass http://127.0.0.1:8080;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_read_timeout 3600s;
    }

    # Static assets with long cache
    location /_next/static/ {
        proxy_pass http://127.0.0.1:3000;
        add_header Cache-Control "public, max-age=31536000, immutable";
    }

    error_log /var/log/nginx/apexintel_error.log;
    access_log /var/log/nginx/apexintel_access.log;
    client_max_body_size 20M;

    # SSL will be added by Certbot
    listen 443 ssl;
    listen [::]:443 ssl;
    # ssl_certificate /etc/letsencrypt/live/starzerp.fi/fullchain.pem;
    # ssl_certificate_key /etc/letsencrypt/live/starzerp.fi/privkey.pem;
}

# HTTP → HTTPS redirect
server {
    listen 80;
    listen [::]:80;
    server_name starzerp.fi www.starzerp.fi;

    if ($host = starzerp.fi) { return 301 https://$host$request_uri; }
    if ($host = www.starzerp.fi) { return 301 https://$host$request_uri; }
    return 404;
}
```

Enable site:
```bash
sudo ln -s /etc/nginx/sites-available/apexintel /etc/nginx/sites-enabled/
# Do NOT remove default or CRM config — they must coexist
sudo nginx -t
sudo systemctl reload nginx
```

### 6.1 SSL Certificate

```bash
sudo certbot --nginx -d starzerp.fi -d www.starzerp.fi
```

---

## 7. LLM Model Setup

### 7.1 Model Weight Transfer

The trained Qwen3-30B-A3B model weights are on the vast.ai instance. Two approaches:

#### Approach A: Quantize on vast.ai, transfer GGUF (~17 GB)

```bash
# On vast.ai instance (has the full fp16 merged model):
ssh -p 39097 -i ~/.ssh/vastai_new root@198.53.64.194

# Install llama.cpp on vast.ai
cd /tmp && git clone https://github.com/ggerganov/llama.cpp.git
cd llama.cpp && cmake -B build -DGGML_CUDA=ON && cmake --build build -j$(nproc)

# Convert merged model to GGUF
python3 convert_hf_to_gguf.py /workspace/outputs/merged_phase1/ \
  --outfile /workspace/outputs/Qwen3-30B-A3B-f16.gguf \
  --outtype f16

# Quantize to Q4_K_M
./build/bin/llama-quantize \
  /workspace/outputs/Qwen3-30B-A3B-f16.gguf \
  /workspace/outputs/Qwen3-30B-A3B-Q4_K_M.gguf \
  Q4_K_M
```

Then transfer to Hetzner:
```bash
# From vast.ai → Hetzner (need SSH key on vast.ai, or use local as jump)
# Option 1: Direct (if vast.ai has Hetzner key)
scp -i /path/to/hetzner-key /workspace/outputs/Qwen3-30B-A3B-Q4_K_M.gguf \
  root@77.42.65.89:/opt/apexintel/model/

# Option 2: Via local machine as jump host
ssh -p 39097 -i ~/.ssh/vastai_new root@198.53.64.194 \
  "cat /workspace/outputs/Qwen3-30B-A3B-Q4_K_M.gguf" | \
  ssh -i ~/.ssh/hetzner-db-mac root@77.42.65.89 \
  "cat > /opt/apexintel/model/Qwen3-30B-A3B-Q4_K_M.gguf"
```

#### Approach B: Transfer full safetensors + quantize on Hetzner

```bash
# Transfer merged model (57 GB) from vast.ai → Hetzner
# Then quantize on Hetzner using CPU (slower but works)
```

### 7.2 Verify Model

```bash
# Quick test
/usr/local/bin/llama-server \
  --model /opt/apexintel/model/Qwen3-30B-A3B-Q4_K_M.gguf \
  --host 127.0.0.1 --port 8081 --ctx-size 512 --threads 4

# Test inference
curl http://127.0.0.1:8081/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "Qwen3-30B-A3B-Q4_K_M",
    "messages": [{"role": "user", "content": "What is Starz Electronics?"}],
    "max_tokens": 100
  }'
```

---

## 8. Full Deployment Procedure (from local machine)

### Step 1: Open Hetzner Firewall

In Hetzner Cloud Console → Firewalls → Add inbound rules for TCP 22, 80, 443.

### Step 2: Initial Server Setup

```bash
ssh -i ~/.ssh/hetzner-db-mac root@77.42.65.89

# Run sections 1.3 through 1.9 from above
# Run section 2 (database setup)
```

### Step 3: Deploy Application Code

```bash
# On local machine:
cd ~/IdeaProjects/ApexIntel

tar czf /tmp/apexintel-deploy.tar.gz \
  --exclude='./target' \
  --exclude='./frontend/node_modules' \
  --exclude='./frontend/.next' \
  --exclude='./.git' \
  --exclude='./training' \
  --exclude='./training_data' \
  --exclude='./docs' \
  .

ls -lh /tmp/apexintel-deploy.tar.gz  # Should be ~5-15 MB

scp -i ~/.ssh/hetzner-db-mac /tmp/apexintel-deploy.tar.gz root@77.42.65.89:/tmp/
```

### Step 4: Build on Server

```bash
ssh -i ~/.ssh/hetzner-db-mac root@77.42.65.89

# Extract source
mkdir -p /opt/apexintel/src
cd /opt/apexintel/src
tar xzf /tmp/apexintel-deploy.tar.gz

# Build Rust (release + LLM)
source $HOME/.cargo/env
cargo build --release -p apex-api --features llm
cargo build --release -p apex-worker --features llm

# Install binaries
cp target/release/apex-api /opt/apexintel/bin/
cp target/release/apex-worker /opt/apexintel/bin/

# Build frontend
cd /opt/apexintel/src/frontend
npm ci
NEXT_PUBLIC_API_BASE_URL=https://starzerp.fi npm run build

# Copy frontend to production path
cp -r /opt/apexintel/src/frontend /opt/apexintel/frontend

# Set ownership
chown -R apexintel:apexintel /opt/apexintel
```

### Step 5: Configure Environment

```bash
# Create .env (see Section 4 above)
nano /opt/apexintel/config/.env
chmod 600 /opt/apexintel/config/.env
chown apexintel:apexintel /opt/apexintel/config/.env
```

### Step 6: Install Systemd Services

```bash
# Copy service files (see Section 5 above) to /etc/systemd/system/
# Then:
systemctl daemon-reload
systemctl enable nats minio apexintel-api apexintel-worker apexintel-frontend
systemctl start nats minio
systemctl start apexintel-api
systemctl start apexintel-worker
systemctl start apexintel-frontend
```

### Step 7: Configure Nginx + SSL

```bash
# Copy nginx config (Section 6) to /etc/nginx/sites-available/apexintel
ln -s /etc/nginx/sites-available/apexintel /etc/nginx/sites-enabled/
nginx -t && systemctl reload nginx

# Get SSL certificate
certbot --nginx -d starzerp.fi -d www.starzerp.fi
```

### Step 8: Transfer & Start LLM Model

```bash
# See Section 7 for model transfer
# After model is in place:
systemctl start apexintel-llm
```

---

## 9. Verification

### 9.1 Service Health

```bash
systemctl status apexintel-api apexintel-worker apexintel-frontend apexintel-llm nats minio postgresql redis
```

### 9.2 API Health Check

```bash
curl -s https://starzerp.fi/api/health | jq
# Expected: {"status":"ok","version":"0.1.0",...}
```

### 9.3 Frontend

```bash
curl -sI https://starzerp.fi | head -5
# Expected: HTTP/2 200
```

### 9.4 LLM

```bash
curl -s http://127.0.0.1:8081/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model":"Qwen3-30B-A3B-Q4_K_M","messages":[{"role":"user","content":"test"}],"max_tokens":10}' | jq
```

---

## 10. Common Operations

```bash
# ─── SSH Access ─────────────────────────────────────────────
ssh -i ~/.ssh/hetzner-db-mac root@77.42.65.89

# ─── Service Management ────────────────────────────────────
sudo systemctl restart apexintel-api
sudo systemctl restart apexintel-worker
sudo systemctl restart apexintel-frontend
sudo systemctl restart apexintel-llm
sudo systemctl status apexintel-api apexintel-worker apexintel-frontend apexintel-llm

# ─── Logs ──────────────────────────────────────────────────
tail -f /opt/apexintel/logs/api.log
tail -f /opt/apexintel/logs/worker.log
tail -f /opt/apexintel/logs/frontend.log
tail -f /opt/apexintel/logs/llm.log
tail -f /var/log/nginx/apexintel_error.log
tail -f /var/log/nginx/apexintel_access.log

# ─── Database ─────────────────────────────────────────────
sudo -u postgres psql apexintel
sudo -u postgres psql apexintel -c "SELECT COUNT(*) FROM companies;"

# ─── Update Code ──────────────────────────────────────────
# On local machine:
scp -i ~/.ssh/hetzner-db-mac /tmp/apexintel-deploy.tar.gz root@77.42.65.89:/tmp/
# On server:
cd /opt/apexintel/src && tar xzf /tmp/apexintel-deploy.tar.gz
cargo build --release -p apex-api --features llm
cp target/release/apex-api /opt/apexintel/bin/
systemctl restart apexintel-api

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

All internal services (PostgreSQL, Redis, NATS, MinIO, llama-server) listen on `127.0.0.1` only — not exposed to the internet.

### 11.3 File Permissions

```bash
chmod 600 /opt/apexintel/config/.env
chmod 700 /opt/apexintel/bin/apex-api /opt/apexintel/bin/apex-worker
chown -R apexintel:apexintel /opt/apexintel
```

---

## 12. Monitoring

### 12.1 Daily Health Check Script

Create `/opt/apexintel/bin/health-check.sh`:

```bash
#!/bin/bash
echo "=== ApexIntel Health Check $(date) ==="

curl -sf http://127.0.0.1:8080/api/health >/dev/null && echo "✅ API" || echo "❌ API"
curl -sf http://127.0.0.1:3000 >/dev/null && echo "✅ Frontend" || echo "❌ Frontend"
redis-cli ping | grep -q PONG && echo "✅ Redis" || echo "❌ Redis"
sudo -u postgres psql apexintel -c "SELECT 1" >/dev/null 2>&1 && echo "✅ PostgreSQL" || echo "❌ PostgreSQL"
curl -sf http://127.0.0.1:8081/health >/dev/null && echo "✅ LLM Server" || echo "❌ LLM Server"

echo "Disk: $(df -h /opt/apexintel | tail -1 | awk '{print $5}')"
echo "Memory: $(free -h | grep Mem | awk '{print $3 "/" $2}')"
```

### 12.2 Cron Jobs

```bash
# Crontab for apexintel user
0 */6 * * * /opt/apexintel/bin/health-check.sh >> /opt/apexintel/logs/health.log 2>&1
0 3 * * * sudo -u postgres pg_dump apexintel | gzip > /opt/apexintel/backups/db_$(date +\%Y\%m\%d).sql.gz
7 3 * * * find /opt/apexintel/backups -name "db_*.sql.gz" -mtime +30 -delete
```

---

## Appendix A – Connection Details

| Service | Address | Notes |
|---------|---------|-------|
| **SSH** | `ssh -i ~/.ssh/hetzner-db-mac root@77.42.65.89` | |
| **Vast.ai** | `ssh -p 39097 -i ~/.ssh/vastai_new root@198.53.64.194` | Model weights source |
| **HTTPS** | `https://starzerp.fi` | Production URL |
| **API** | `http://127.0.0.1:8080` (internal) | Axum server |
| **Frontend** | `http://127.0.0.1:3000` (internal) | Next.js |
| **LLM** | `http://127.0.0.1:8081` (internal) | llama-server |
| **PostgreSQL** | `127.0.0.1:5432` | DB: `apexintel` |
| **Redis** | `127.0.0.1:6379` | |
| **NATS** | `127.0.0.1:4222` | |
| **MinIO** | `127.0.0.1:9000` (API), `127.0.0.1:9001` (console) | |

## Appendix B – Model Weights Inventory (vast.ai)

| Path | Size | Purpose |
|------|------|---------|
| `merged_phase1/` | 57 GB | Full fp16 model (2× safetensors) |
| `phase2_sft/best_adapter_v3/` | 828 MB | LoRA adapter (Phase 2 SFT) |
| `phase1_dapt/` | 1.1 GB | Phase 1 DAPT adapter |

The `merged_phase1/` already has Phase 1 merged. Phase 2 best adapter needs to be merged on top, then the result quantized to GGUF Q4_K_M for deployment.

## Appendix C – Deployment Status

- [x] Deployment guide created
- [ ] Hetzner firewall opened (ports 22, 80, 443)
- [ ] Server setup (packages, users, directories)
- [ ] PostgreSQL + Redis configured
- [ ] NATS + MinIO installed
- [ ] Rust toolchain installed
- [ ] Node.js 20 installed
- [ ] Application code deployed and built
- [ ] Environment configured (.env)
- [ ] Systemd services installed
- [ ] Nginx configured
- [ ] SSL certificate obtained
- [ ] Model quantized (GGUF Q4_K_M)
- [ ] Model transferred to server
- [ ] llama-server running with model
- [ ] Full system verified
