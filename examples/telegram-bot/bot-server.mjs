// Multi-tenant Telegram bot backed by shot.
//
// Architecture:
//   Node.js long-polls Telegram's getUpdates.
//   Each incoming message spawns a shot container.
//   Each chat gets its own user_data/<chat_id>/ directory, mounted into
//   the container, so session history and any written files persist per
//   user. API key comes from the host env; shot reads it via the
//   SHOT_CONFIG_GEMINI_API_KEY env var.
//
// Requirements:
//   Node.js 18+
//   Docker with affixpin/shot:latest pulled (or set SHOT_IMAGE to another tag)
//   TELEGRAM_TOKEN and GEMINI_API_KEY exported in the host env.

import { spawn } from "node:child_process";
import { mkdirSync, existsSync } from "node:fs";
import { join } from "node:path";

const TELEGRAM_TOKEN = process.env.TELEGRAM_TOKEN;
const GEMINI_API_KEY = process.env.GEMINI_API_KEY;
const DATA_DIR = join(process.cwd(), "user_data");
const IMAGE = process.env.SHOT_IMAGE || "affixpin/shot:latest";
const MEMORY_LIMIT = process.env.SHOT_MEMORY || "128m";
const CPU_LIMIT = process.env.SHOT_CPUS || "0.5";
const POLL_TIMEOUT = 30;

if (!TELEGRAM_TOKEN || !GEMINI_API_KEY) {
  console.error("Set TELEGRAM_TOKEN and GEMINI_API_KEY in the environment");
  process.exit(1);
}

if (!existsSync(DATA_DIR)) mkdirSync(DATA_DIR, { recursive: true });

console.log(`shot telegram bot polling (image: ${IMAGE}, data: ${DATA_DIR})`);

// Run shot in a one-shot container. Returns trimmed stdout or rejects.
function runShot(userDir, chatId, message) {
  return new Promise((resolve, reject) => {
    const args = [
      "run", "--rm",
      "-e", `SHOT_CONFIG_GEMINI_API_KEY=${GEMINI_API_KEY}`,
      "-v", `${userDir}:/home/agent/.local/share/shot`,
      "--memory", MEMORY_LIMIT,
      "--cpus", CPU_LIMIT,
      IMAGE,
      "--quiet",
      "--tools",
      `--session=${chatId}`,
      message,
    ];

    const proc = spawn("docker", args, { stdio: ["ignore", "pipe", "pipe"] });
    let out = "";
    let err = "";
    proc.stdout.on("data", (d) => { out += d.toString(); });
    proc.stderr.on("data", (d) => { err += d.toString(); });
    proc.on("error", reject);
    proc.on("close", (code) => {
      if (code === 0) resolve(out.trim());
      else reject(new Error(err.trim() || `shot exited with code ${code}`));
    });
  });
}

async function sendTelegramMessage(chatId, text) {
  try {
    await fetch(`https://api.telegram.org/bot${TELEGRAM_TOKEN}/sendMessage`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ chat_id: chatId, text }),
    });
  } catch (e) {
    console.error("failed to send telegram message:", e);
  }
}

async function handleUpdate(update) {
  const message = update.message;
  if (!message?.text) return;

  const chatId = String(message.chat.id);
  const text = message.text;
  const userDir = join(DATA_DIR, chatId);

  if (!existsSync(userDir)) mkdirSync(userDir, { recursive: true });

  console.log(`[${chatId}] user: ${text}`);

  try {
    const reply = await runShot(userDir, chatId, text);
    const body = reply || "(no output)";
    console.log(`[${chatId}] shot: ${body}`);
    await sendTelegramMessage(chatId, body);
  } catch (e) {
    console.error(`[${chatId}] error:`, e.message);
    await sendTelegramMessage(chatId, "error: could not process your message");
  }
}

// Long-polling loop.
let lastUpdateId = 0;
while (true) {
  try {
    const url = `https://api.telegram.org/bot${TELEGRAM_TOKEN}/getUpdates?offset=${lastUpdateId + 1}&timeout=${POLL_TIMEOUT}`;
    const response = await fetch(url);
    const data = await response.json();

    if (data.ok && data.result.length > 0) {
      for (const update of data.result) {
        lastUpdateId = update.update_id;
        // Fire-and-forget so concurrent users don't block each other.
        handleUpdate(update).catch((e) => console.error("handleUpdate:", e));
      }
    }
  } catch (e) {
    console.error("polling error:", e.message);
    await new Promise((r) => setTimeout(r, 5000));
  }
}
