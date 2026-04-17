# shot multitenant telegram agent

Telegram bot where each chat gets its own shot container. Node.js polls Telegram, spawns `affixpin/shot:latest` per message, mounts `user_data/<chat_id>/` into the container so sessions persist.

## run

```
export TELEGRAM_TOKEN=...
export GEMINI_API_KEY=...
npm install
npm start
```

## knobs

| env           | default                  |
|---------------|--------------------------|
| `SHOT_IMAGE`  | `affixpin/shot:latest`   |
| `SHOT_MEMORY` | `128m`                   |
| `SHOT_CPUS`   | `0.5`                    |
