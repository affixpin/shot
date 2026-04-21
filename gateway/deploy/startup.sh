#!/bin/bash
# Runs on VM boot via GCE startup-script metadata.
# Installs Docker, fetches secrets from Secret Manager, runs the gateway
# container. Everything else (per-chat template bootstrap, tool overrides,
# skill bundling) now lives inside the gateway itself.
set -euo pipefail

export DEBIAN_FRONTEND=noninteractive

# ── Docker CE from docker.com ──────────────────────────────────────────
apt-get update
apt-get install -y ca-certificates curl gnupg
apt-get remove -y docker.io docker-doc docker-compose podman-docker containerd runc 2>/dev/null || true
install -m 0755 -d /etc/apt/keyrings
if [ ! -f /etc/apt/keyrings/docker.asc ]; then
  curl -fsSL https://download.docker.com/linux/debian/gpg -o /etc/apt/keyrings/docker.asc
  chmod a+r /etc/apt/keyrings/docker.asc
fi
. /etc/os-release
cat > /etc/apt/sources.list.d/docker.list <<EOF
deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/debian $VERSION_CODENAME stable
EOF
apt-get update
apt-get install -y docker-ce docker-ce-cli containerd.io
systemctl enable --now docker

docker pull affixpin/shot:latest         || true
docker pull affixpin/shot-gateway:latest || true
docker pull caddy:2-alpine               || true
docker pull postgres:16-alpine           || true
docker pull redis:7-alpine               || true
docker pull nangohq/nango-server:hosted  || true

# ── Per-chat data (writable, mounted into each spawned shot container) ─
mkdir -p /opt/shot-data
chown 1000:1000 /opt/shot-data

# ── Caddy reverse proxy on 80/443 (auto TLS via Let's Encrypt) ──────────
# bot.autoshot.dev   → gateway webapp port 4000 (Telegram Mini App + webhooks)
# nango.autoshot.dev → Nango server port 3003
mkdir -p /opt/caddy
cat > /opt/caddy/Caddyfile <<'CADDY'
bot.autoshot.dev {
    handle /health {
        respond "ok" 200
    }
    reverse_proxy 127.0.0.1:4000
}

nango.autoshot.dev {
    reverse_proxy 127.0.0.1:3003
}
CADDY

docker rm -f shot-caddy 2>/dev/null || true
docker run -d --name shot-caddy --restart=always \
  --network host \
  -v /opt/caddy/Caddyfile:/etc/caddy/Caddyfile:ro \
  -v /opt/caddy/data:/data \
  -v /opt/caddy/config:/config \
  caddy:2-alpine

# ── Nango (OAuth + proxy for 700+ APIs) ────────────────────────────────
# Runs as its own docker-compose stack so the pieces stay isolated from
# the gateway. Free tier: we get auth + proxy; sync logic lives in our
# worker (see sync-worker/).
mkdir -p /opt/nango
cat > /opt/nango/docker-compose.yml <<'YAML'
services:
  nango-db:
    image: postgres:16-alpine
    restart: always
    environment:
      POSTGRES_USER: nango
      POSTGRES_PASSWORD: nango
      POSTGRES_DB: nango
    volumes:
      - ./pgdata:/var/lib/postgresql/data
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U nango"]
      interval: 5s
      retries: 10

  nango-redis:
    image: redis:7-alpine
    restart: always

  nango-server:
    image: nangohq/nango-server:hosted
    platform: linux/amd64
    restart: always
    depends_on:
      nango-db:
        condition: service_healthy
    environment:
      NANGO_ENCRYPTION_KEY: "${NANGO_ENCRYPTION_KEY}"
      NANGO_DB_USER: nango
      NANGO_DB_PASSWORD: nango
      NANGO_DB_HOST: nango-db
      NANGO_DB_PORT: "5432"
      NANGO_DB_NAME: nango
      NANGO_DB_SSL: "false"
      NANGO_SERVER_URL: "https://nango.autoshot.dev"
      NANGO_PUBLIC_SERVER_URL: "https://nango.autoshot.dev"
      FLAG_SERVE_CONNECT_UI: "true"
      FLAG_AUTH_ENABLED: "true"
      NANGO_DASHBOARD_USERNAME: admin
      NANGO_DASHBOARD_PASSWORD: "${NANGO_DASHBOARD_PASSWORD}"
      LOG_LEVEL: info
    ports:
      - "127.0.0.1:3003:8080"
      - "127.0.0.1:3009:3009"
YAML

cat > /opt/nango/.env <<ENV
NANGO_ENCRYPTION_KEY=$(gcloud secrets versions access latest --secret=nango-encryption-key)
NANGO_DASHBOARD_PASSWORD=$(gcloud secrets versions access latest --secret=nango-admin-password)
ENV
chmod 600 /opt/nango/.env

(cd /opt/nango && docker compose --env-file .env up -d)

# ── Secrets ────────────────────────────────────────────────────────────
TELEGRAM_TOKEN=$(gcloud secrets versions access latest --secret=telegram-token)
GEMINI_API_KEY=$(gcloud secrets versions access latest --secret=gemini-api-key)
JINA_API_KEY=$(gcloud secrets versions access latest --secret=jina-api-key 2>/dev/null || echo "")

# ── Run the gateway container ──────────────────────────────────────────
docker rm -f shot-gateway 2>/dev/null || true
docker run -d --name shot-gateway --restart=always \
  --network host \
  -e TELEGRAM_TOKEN="$TELEGRAM_TOKEN" \
  -e GEMINI_API_KEY="$GEMINI_API_KEY" \
  -e JINA_API_KEY="$JINA_API_KEY" \
  -e DATA_DIR=/opt/shot-data \
  -v /var/run/docker.sock:/var/run/docker.sock \
  -v /opt/shot-data:/opt/shot-data \
  affixpin/shot-gateway:latest
