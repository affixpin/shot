import { spawnSync } from "bun";
import { join } from "node:path";
import { mkdirSync, existsSync } from "node:fs";

/**
 * Shot Multi-tenant Telegram Bot Server
 * 
 * Requirements:
 * 1. Bun installed (bun install grammy)
 * 2. Docker image 'shot:latest' built from root Dockerfile
 * 3. Environment variables: TELEGRAM_TOKEN, GEMINI_API_KEY
 */

const TOKEN = Bun.env.TELEGRAM_TOKEN;
const API_KEY = Bun.env.GEMINI_API_KEY;
const DATA_DIR = join(process.cwd(), "user_data");

if (!TOKEN || !API_KEY) {
  console.error("Missing TELEGRAM_TOKEN or GEMINI_API_KEY");
  process.exit(1);
}

// Ensure base data directory exists
if (!existsSync(DATA_DIR)) mkdirSync(DATA_DIR, { recursive: true });

console.log("🚀 Shot Bot Server starting (Polling)...");

async function handleUpdate(update: any) {
  const message = update.message;
  if (!message?.text) return;

  const chatId = message.chat.id.toString();
  const userDir = join(DATA_DIR, chatId);
  const text = message.text;

  // Ensure user isolation folder exists
  if (!existsSync(userDir)) mkdirSync(userDir, { recursive: true });

  console.log(`[${chatId}] User: ${text}`);

  /**
   * Run Shot in Docker
   * - Mount user folder to ~/.local/share/shot
   * - Use --rm to cleanup container after exit
   * - Inject Gemini API Key
   */
  const dockerArgs = [
    "run", "--rm",
    "-e", `GEMINI_API_KEY=${API_KEY}`,
    "-v", `${userDir}:/root/.local/share/shot`,
    "--memory", "512m",
    "--cpus", "0.5",
    "shot:latest",
    "shot", text
  ];

  // If first time (no config), run configuration first
  if (!existsSync(join(userDir, "agent.toml"))) {
      console.log(`[${chatId}] First run: Configuring...`);
      spawnSync(["docker", "run", "--rm", 
        "-v", `${userDir}:/root/.local/share/shot`, 
        "shot:latest", "shot", "configure", "gemini", API_KEY]);
  }

  try {
    const proc = spawnSync(["docker", ...dockerArgs]);
    const output = proc.stdout.toString().trim() || proc.stderr.toString().trim();
    
    await sendTelegramMessage(chatId, output || "Shot executed but returned no output.");
  } catch (err) {
    console.error(`[${chatId}] Error:`, err);
    await sendTelegramMessage(chatId, "❌ Failed to execute Shot.");
  }
}

async function sendTelegramMessage(chatId: string, text: string) {
  try {
    await fetch(`https://api.telegram.org/bot${TOKEN}/sendMessage`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ chat_id: chatId, text: text })
    });
  } catch (e) {
    console.error("Failed to send TG message:", e);
  }
}

// Simple Polling Loop
let lastUpdateId = 0;
while (true) {
  try {
    const response = await fetch(`https://api.telegram.org/bot${TOKEN}/getUpdates?offset=${lastUpdateId + 1}&timeout=30`);
    const data = await response.json() as any;

    if (data.ok && data.result.length > 0) {
      for (const update of data.result) {
        lastUpdateId = update.update_id;
        handleUpdate(update); // Fire and forget to handle multiple users
      }
    }
  } catch (e) {
    console.error("Polling error:", e);
    await new Promise(r => setTimeout(r, 5000));
  }
}
