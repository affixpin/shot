#!/bin/bash
# Runs on VM boot via GCE startup-script metadata.
# Installs Docker, bootstraps the shared shot-template directory,
# fetches secrets from Secret Manager, runs the gateway container.
set -euo pipefail

export DEBIAN_FRONTEND=noninteractive

# ── Clean up legacy systemd-based deploy (pre-gateway-image path) ──────
# Previous versions ran the bot under a systemd unit that binds port 3000.
# Remove it so the gateway container can take over that port.
systemctl stop    shot-bot 2>/dev/null || true
systemctl disable shot-bot 2>/dev/null || true
rm -f /etc/systemd/system/shot-bot.service /etc/shot-bot.env
systemctl daemon-reload

# ── Dependencies ───────────────────────────────────────────────────────
apt-get update
apt-get install -y docker.io ca-certificates
systemctl enable --now docker

docker pull affixpin/shot:latest          || true
docker pull affixpin/shot-gateway:latest  || true

# ── Shared shot template (tools + soul + skills) ───────────────────────
# Bootstrap by running shot's built-in default extraction once, then
# overwrite web_search/web_read with the versions that route through
# the gateway's Jina proxy (so API keys stay server-side).
mkdir -p /opt/shot-template
chown 1000:1000 /opt/shot-template
docker run --rm \
  -v /opt/shot-template:/home/agent/.local/share/shot \
  -e SHOT_CONFIG_GEMINI_API_KEY=placeholder \
  affixpin/shot:latest tools >/dev/null 2>&1 || true
mkdir -p /opt/shot-template/tools

cat > /opt/shot-template/tools/web_search.toml <<'EOF'
healthcheck = "which curl"
name = "web_search"
description = "Search the web and return results as markdown"
command = '''curl -sS -G "http://host.docker.internal:3000/jina/search/" --data-urlencode "q=$query" -H "Accept: text/markdown"'''

[vars.query]
type = "string"
description = "Search query"
required = true
EOF

cat > /opt/shot-template/tools/web_read.toml <<'EOF'
healthcheck = "which curl"
name = "web_read"
description = "Read a web page and extract its content as markdown"
command = 'curl -sS "http://host.docker.internal:3000/jina/read/$url" -H "Accept: text/markdown"'

[vars.url]
type = "string"
description = "URL to read"
required = true
EOF

chown -R 1000:1000 /opt/shot-template

# Per-chat writable data (sessions, tasks.md, etc.).
mkdir -p /opt/shot-data
chown 1000:1000 /opt/shot-data

# ── Secrets ────────────────────────────────────────────────────────────
TELEGRAM_TOKEN=$(gcloud secrets versions access latest --secret=telegram-token)
GEMINI_API_KEY=$(gcloud secrets versions access latest --secret=gemini-api-key)
JINA_API_KEY=$(gcloud secrets versions access latest --secret=jina-api-key 2>/dev/null || echo "")

# ── Run the gateway container ──────────────────────────────────────────
# --network host so the internal proxy (port 3000) is reachable from
# spawned shot containers via host.docker.internal.
# /var/run/docker.sock mount lets the gateway spawn shot containers
# on the host daemon. Shared paths use the same names inside and out
# so -v args the gateway generates remain valid host paths.
docker rm -f shot-gateway 2>/dev/null || true
docker run -d --name shot-gateway --restart=always \
  --network host \
  -e TELEGRAM_TOKEN="$TELEGRAM_TOKEN" \
  -e GEMINI_API_KEY="$GEMINI_API_KEY" \
  -e JINA_API_KEY="$JINA_API_KEY" \
  -e DATA_DIR=/opt/shot-data \
  -e SHOT_TEMPLATE_DIR=/opt/shot-template \
  -v /var/run/docker.sock:/var/run/docker.sock \
  -v /opt/shot-data:/opt/shot-data \
  -v /opt/shot-template:/opt/shot-template:ro \
  affixpin/shot-gateway:latest
