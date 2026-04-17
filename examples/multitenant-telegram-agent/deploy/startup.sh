#!/bin/bash
# Runs on VM first boot via GCE startup-script metadata.
# Installs Node + Docker, bootstraps a shared shot template (tools + soul)
# with the web_search/web_read tomls rewritten to hit our Jina proxy,
# fetches secrets from Secret Manager, starts the bot under systemd.
set -euo pipefail

export DEBIAN_FRONTEND=noninteractive

# ── Dependencies ───────────────────────────────────────────────────────
apt-get update
curl -fsSL https://deb.nodesource.com/setup_22.x | bash -
apt-get install -y nodejs docker.io git ca-certificates
systemctl enable --now docker

docker pull affixpin/shot:latest || true

# ── Shared shot template (tools + soul) ────────────────────────────────
# Run shot once with a placeholder key so Config::load triggers its
# first-run bootstrap and writes the 11 default tool tomls + SOUL.md
# into /opt/shot-template. We then overwrite the two tools that would
# otherwise ship a Jina API key into the container.
#
# The mount must be owned by uid 1000 before the container runs,
# because shot runs as user `agent` (uid 1000) inside the image and
# can't write to a root-owned mount.
mkdir -p /opt/shot-template
chown 1000:1000 /opt/shot-template
docker run --rm \
  -v /opt/shot-template:/home/agent/.local/share/shot \
  -e SHOT_CONFIG_GEMINI_API_KEY=placeholder \
  affixpin/shot:latest tools >/dev/null 2>&1 || true
mkdir -p /opt/shot-template/tools  # safety in case bootstrap silently failed

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

# ── App code ───────────────────────────────────────────────────────────
rm -rf /opt/shot
git clone https://github.com/affixpin/shot /opt/shot
APP_DIR=/opt/shot/examples/multitenant-telegram-agent
cd "$APP_DIR"
npm install --include=dev

# ── Secrets ────────────────────────────────────────────────────────────
TELEGRAM_TOKEN=$(gcloud secrets versions access latest --secret=telegram-token)
GEMINI_API_KEY=$(gcloud secrets versions access latest --secret=gemini-api-key)
JINA_API_KEY=$(gcloud secrets versions access latest --secret=jina-api-key 2>/dev/null || echo "")

install -o root -g root -m 600 /dev/null /etc/shot-bot.env
cat > /etc/shot-bot.env <<EOF
TELEGRAM_TOKEN=$TELEGRAM_TOKEN
GEMINI_API_KEY=$GEMINI_API_KEY
JINA_API_KEY=$JINA_API_KEY
EOF

# ── systemd ────────────────────────────────────────────────────────────
install -m 644 "$APP_DIR/deploy/shot-bot.service" /etc/systemd/system/shot-bot.service
systemctl daemon-reload
systemctl enable shot-bot
# restart (not start) so reruns pick up fresh code / env / secrets
systemctl restart shot-bot
