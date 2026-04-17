import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { mkdirSync } from "node:fs";
import { join } from "node:path";

const exec = promisify(execFile);

const { TELEGRAM_TOKEN, GEMINI_API_KEY } = process.env;
if (!TELEGRAM_TOKEN || !GEMINI_API_KEY) {
  console.error("set TELEGRAM_TOKEN and GEMINI_API_KEY");
  process.exit(1);
}

const DATA = join(process.cwd(), "user_data");
const IMAGE = process.env.SHOT_IMAGE ?? "affixpin/shot:latest";
const MEMORY = process.env.SHOT_MEMORY ?? "128m";
const CPUS = process.env.SHOT_CPUS ?? "0.5";

mkdirSync(DATA, { recursive: true });

type Update = {
  update_id: number;
  message?: { text?: string; chat: { id: number } };
};

const tg = (method: string, body: object) =>
  fetch(`https://api.telegram.org/bot${TELEGRAM_TOKEN}/${method}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });

async function handle({ message }: Update) {
  if (!message?.text) return;
  const chat_id = String(message.chat.id);
  const userDir = join(DATA, chat_id);
  mkdirSync(userDir, { recursive: true });
  console.log(`[${chat_id}] ${message.text}`);
  try {
    const { stdout } = await exec("docker", [
      "run", "--rm",
      "-e", `SHOT_CONFIG_GEMINI_API_KEY=${GEMINI_API_KEY}`,
      "-v", `${userDir}:/home/agent/.local/share/shot`,
      "--memory", MEMORY, "--cpus", CPUS,
      IMAGE, "--quiet", "--tools", `--session=${chat_id}`, message.text,
    ], { maxBuffer: 10 * 1024 * 1024, timeout: 120_000 });
    await tg("sendMessage", { chat_id, text: stdout.trim() || "(no output)" });
  } catch (e: any) {
    console.error(`[${chat_id}]`, e.message);
    await tg("sendMessage", { chat_id, text: "error processing your message" });
  }
}

console.log(`shot telegram bot polling (image: ${IMAGE})`);

let offset = 0;
while (true) {
  try {
    const r = await fetch(
      `https://api.telegram.org/bot${TELEGRAM_TOKEN}/getUpdates?offset=${offset + 1}&timeout=30`,
    );
    const { ok, result } = (await r.json()) as { ok: boolean; result: Update[] };
    if (!ok) continue;
    for (const u of result) {
      offset = u.update_id;
      handle(u).catch(console.error);
    }
  } catch (e: any) {
    console.error("poll:", e.message);
    await new Promise((r) => setTimeout(r, 5000));
  }
}
