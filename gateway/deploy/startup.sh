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

# ── Per-chat data (writable, mounted into each spawned shot container) ─
mkdir -p /opt/shot-data
chown 1000:1000 /opt/shot-data

# ── Caddy reverse proxy on 80/443 (auto TLS via Let's Encrypt) ──────────
# bot.autoshot.dev → gateway webapp port 4000 (future webhooks, health)
mkdir -p /opt/caddy
cat > /opt/caddy/Caddyfile <<'CADDY'
bot.autoshot.dev {
    handle /health {
        respond "ok" 200
    }
    reverse_proxy 127.0.0.1:4000
}
CADDY

docker rm -f shot-caddy 2>/dev/null || true
docker run -d --name shot-caddy --restart=always \
  --network host \
  -v /opt/caddy/Caddyfile:/etc/caddy/Caddyfile:ro \
  -v /opt/caddy/data:/data \
  -v /opt/caddy/config:/config \
  caddy:2-alpine

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
