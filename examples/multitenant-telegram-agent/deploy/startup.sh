#!/bin/bash
# Runs once on VM first boot via GCE instance metadata.
# Installs Node.js + Docker, clones the repo, fetches secrets from
# Secret Manager, starts the bot under systemd.
set -euo pipefail

export DEBIAN_FRONTEND=noninteractive

# Node.js 22 + Docker
apt-get update
curl -fsSL https://deb.nodesource.com/setup_22.x | bash -
apt-get install -y nodejs docker.io git ca-certificates
systemctl enable --now docker

# Pre-pull the shot image so the first message is snappy
docker pull affixpin/shot:latest || true

# App code
rm -rf /opt/shot
git clone https://github.com/affixpin/shot /opt/shot
APP_DIR=/opt/shot/examples/multitenant-telegram-agent
cd "$APP_DIR"
npm install --include=dev

# Secrets — fetched from Secret Manager, written to a root-only env file
TELEGRAM_TOKEN=$(gcloud secrets versions access latest --secret=telegram-token)
GEMINI_API_KEY=$(gcloud secrets versions access latest --secret=gemini-api-key)
install -o root -g root -m 600 /dev/null /etc/shot-bot.env
cat > /etc/shot-bot.env <<EOF
TELEGRAM_TOKEN=$TELEGRAM_TOKEN
GEMINI_API_KEY=$GEMINI_API_KEY
EOF

# systemd unit
install -m 644 "$APP_DIR/deploy/shot-bot.service" /etc/systemd/system/shot-bot.service
systemctl daemon-reload
systemctl enable --now shot-bot
