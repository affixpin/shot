# shot telegram bot

Multi-tenant Telegram bot backed by shot. Each chat gets its own session and filesystem, isolated in its own container.

## how it works

Node.js long-polls Telegram. Each message spawns a one-shot docker container running `affixpin/shot:latest` with the user's data directory mounted in. Shot handles the agent loop, writes session state to the mounted volume, exits. The next message from the same chat resumes the session.

No daemon, no message queue, no database. Just Node + Docker + shot.

## requirements

- Node.js 18 or newer
- Docker (will pull `affixpin/shot:latest` on first run)
- A Telegram bot token (from [@BotFather](https://t.me/BotFather))
- A Gemini API key ([aistudio.google.com/apikey](https://aistudio.google.com/apikey))

## run

```bash
export TELEGRAM_TOKEN=your_bot_token
export GEMINI_API_KEY=your_gemini_key
npm start
```

Message your bot on Telegram. Each user's state lands in `user_data/<chat_id>/`.

## knobs

Set these in the environment if you want to tune behaviour:

| Variable       | Default                    | What it does                         |
|----------------|----------------------------|--------------------------------------|
| `SHOT_IMAGE`   | `affixpin/shot:latest`     | Which shot image to run              |
| `SHOT_MEMORY`  | `128m`                     | Per-container RAM limit              |
| `SHOT_CPUS`    | `0.5`                      | Per-container CPU limit              |

## notes

- `--tools` enables all the default tools (file_read, shell, web_search, etc.). Remove that flag in `bot-server.mjs` if you want a chat-only bot.
- Sessions persist under `user_data/<chat_id>/sessions/<chat_id>.db`. Delete that file to reset a user's history.
- The bot uses long-polling, not webhooks, so it works anywhere without a public URL.
- Concurrent users run in parallel — `handleUpdate` is fire-and-forget and each spawns its own container.
