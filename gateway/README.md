# shot-gateway

Long-running host for short-lived shot agents.

Gateway is a tiny Node.js server that:

- polls Telegram (long-polling, no webhook required)
- spawns one `affixpin/shot:latest` container **per message**, isolated via a per-chat data directory
- runs a local HTTP proxy so `shot` containers never see your third-party API keys (Gemini, Jina) — the keys stay in the gateway process
- streams each shot event (`tool.call`, `tool.result`, `llm.response`) back to Telegram as its own message so users watch the agent work in real time

Published as `affixpin/shot-gateway:latest`.

## one-command run

```bash
docker run -d --name shot-gateway --restart=always \
  --network host \
  -e TELEGRAM_TOKEN=...      `# @BotFather` \
  -e GEMINI_API_KEY=...      `# https://aistudio.google.com/apikey` \
  -e JINA_API_KEY=...        `# optional; https://jina.ai/reader` \
  -e DATA_DIR=/opt/shot-data \
  -e SHOT_TEMPLATE_DIR=/opt/shot-template \
  -v /var/run/docker.sock:/var/run/docker.sock \
  -v /opt/shot-data:/opt/shot-data \
  -v /opt/shot-template:/opt/shot-template:ro \
  affixpin/shot-gateway:latest
```

Required volumes explained:

| mount | why |
|---|---|
| `/var/run/docker.sock` | so gateway can spawn `shot` containers via the host Docker daemon |
| `/opt/shot-data` | per-chat writable directory, mounted identically in and out so `-v` args to spawned shot containers reference valid host paths |
| `/opt/shot-template` | shared read-only tools + SOUL + skills for every spawned shot container (populated by the provisioning step below) |

`--network host` is used so the internal proxy on port 3000 is reachable from spawned shot containers via `host.docker.internal`.

## first-run setup

Before the gateway starts, populate `/opt/shot-template` with shot's defaults and overwrite the two web tools so they route through the proxy:

```bash
mkdir -p /opt/shot-template && chown 1000:1000 /opt/shot-template
docker run --rm \
  -v /opt/shot-template:/home/agent/.local/share/shot \
  -e SHOT_CONFIG_GEMINI_API_KEY=placeholder \
  affixpin/shot:latest tools
# Then overwrite web_search.toml / web_read.toml to use the proxy.
# See deploy/startup.sh for the exact toml snippets.
```

`deploy/startup.sh` does all of this in one pass — recommended for any real host.

## env vars

| env                  | default                    | what                                               |
|----------------------|----------------------------|----------------------------------------------------|
| `TELEGRAM_TOKEN`     | (required)                 | @BotFather bot token                               |
| `GEMINI_API_KEY`     | (required)                 | proxy uses this to authenticate to Gemini          |
| `JINA_API_KEY`       | optional                   | enables `web_search` / `web_read`                  |
| `DATA_DIR`           | `./user_data`              | per-chat writable directory                        |
| `SHOT_TEMPLATE_DIR`  | `/opt/shot-template`       | shared read-only tools / soul / skills             |
| `SHOT_IMAGE`         | `affixpin/shot:latest`     | which shot image to spawn                          |
| `SHOT_MEMORY`        | `128m`                     | per-container RAM cap                              |
| `SHOT_CPUS`          | `0.5`                      | per-container CPU cap                              |
| `PROXY_PORT`         | `3000`                     | internal HTTP proxy port                           |

## run without docker (local dev)

```bash
export TELEGRAM_TOKEN=...
export GEMINI_API_KEY=...
export JINA_API_KEY=...     # optional
npm install
npm start
```

Gateway will use the local Docker daemon to spawn shot containers, and `./user_data` for per-chat state.

## deploy to gcp

`deploy/deploy.sh` provisions a free-tier `e2-micro`, Secret Manager secrets, and service-account bindings. `deploy/startup.sh` runs on VM boot and handles everything above.

```bash
# One-time infrastructure
./deploy/deploy.sh

# Populate secrets
echo -n "TELEGRAM_BOT_TOKEN" | gcloud secrets versions add telegram-token --data-file=-
echo -n "GEMINI_API_KEY"     | gcloud secrets versions add gemini-api-key --data-file=-
echo -n "JINA_API_KEY"       | gcloud secrets versions add jina-api-key   --data-file=-

# Start the bot
gcloud compute instances reset shot-bot --zone=us-central1-a

# Logs
gcloud compute ssh shot-bot --zone=us-central1-a -- docker logs -f shot-gateway
```
