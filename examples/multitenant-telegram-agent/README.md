# shot multitenant telegram agent

Telegram bot where each chat gets its own shot container. Node.js polls Telegram, spawns `affixpin/shot:latest` per message, mounts `user_data/<chat_id>/` into the container so sessions persist per user.

A provider proxy runs inside the Node.js server. Shot containers hit the proxy (via `host.docker.internal`) instead of Gemini or Jina directly, so no third-party API keys ever enter a container and can't be exfiltrated through a tool call. Path routing:

- `/gemini/*` → Gemini chat completions
- `/jina/search/*` → Jina search
- `/jina/read/*` → Jina reader

The `web_search` and `web_read` tool definitions are overwritten on the VM to point at the proxy paths; default shot tool tomls are otherwise untouched.

## local

```
export TELEGRAM_TOKEN=...
export GEMINI_API_KEY=...
export JINA_API_KEY=...   # optional; enables web_search and web_read
npm install
npm start
```

## knobs

| env           | default                  |
|---------------|--------------------------|
| `SHOT_IMAGE`  | `affixpin/shot:latest`   |
| `SHOT_MEMORY` | `128m`                   |
| `SHOT_CPUS`   | `0.5`                    |
| `PROXY_PORT`  | `3000`                   |

## deploy to gcp

Single-VM deployment on a free-tier `e2-micro` with secrets in Secret Manager. The VM runs a startup script that installs Docker + Node, fetches secrets, and starts the bot under systemd.

```
# One-time setup (creates secrets, service account, VM)
./deploy/deploy.sh

# Add your secret values
echo -n "TELEGRAM_BOT_TOKEN" | gcloud secrets versions add telegram-token --data-file=-
echo -n "GEMINI_API_KEY"     | gcloud secrets versions add gemini-api-key --data-file=-
echo -n "JINA_API_KEY"       | gcloud secrets versions add jina-api-key   --data-file=-

# Reboot so startup.sh picks up the new secret versions
gcloud compute instances reset shot-bot --zone=us-central1-a

# Tail logs
gcloud compute ssh shot-bot --zone=us-central1-a -- sudo journalctl -u shot-bot -f
```

Env overrides you can set for the deploy:

| env        | default              |
|------------|----------------------|
| `PROJECT`  | current gcloud project |
| `ZONE`     | `us-central1-a`      |
| `NAME`     | `shot-bot`           |
