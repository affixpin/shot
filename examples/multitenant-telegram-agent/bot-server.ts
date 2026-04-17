import { spawn } from "node:child_process";
import { mkdirSync, chownSync } from "node:fs";
import { join } from "node:path";
import { createServer } from "node:http";
import { createInterface } from "node:readline";

const { TELEGRAM_TOKEN, GEMINI_API_KEY } = process.env;
if (!TELEGRAM_TOKEN || !GEMINI_API_KEY) {
  console.error("set TELEGRAM_TOKEN and GEMINI_API_KEY");
  process.exit(1);
}

const DATA = join(process.cwd(), "user_data");
const IMAGE = process.env.SHOT_IMAGE ?? "affixpin/shot:latest";
const MEMORY = process.env.SHOT_MEMORY ?? "128m";
const CPUS = process.env.SHOT_CPUS ?? "0.5";
const PROXY_PORT = Number(process.env.PROXY_PORT ?? 3000);
const GEMINI_BASE = "https://generativelanguage.googleapis.com/v1beta/openai";

mkdirSync(DATA, { recursive: true });

type Update = {
  update_id: number;
  message?: { text?: string; chat: { id: number } };
};

// LLM proxy. Shot talks here instead of Gemini directly so the real key
// never enters a container and can't be exfiltrated via a tool call.
createServer(async (req, res) => {
  try {
    if (req.method !== "POST" || !req.url) {
      res.writeHead(404).end();
      return;
    }
    const body: Buffer[] = [];
    for await (const chunk of req) body.push(chunk);
    const upstream = await fetch(`${GEMINI_BASE}${req.url}`, {
      method: "POST",
      headers: {
        "content-type": "application/json",
        authorization: `Bearer ${GEMINI_API_KEY}`,
      },
      body: Buffer.concat(body),
    });
    res.writeHead(upstream.status, {
      "content-type": upstream.headers.get("content-type") ?? "application/json",
    });
    if (upstream.body) {
      for await (const chunk of upstream.body) res.write(chunk);
    }
    res.end();
  } catch (e: any) {
    console.error("proxy:", e.message);
    if (!res.headersSent) res.writeHead(502);
    res.end();
  }
}).listen(PROXY_PORT, "0.0.0.0", () => {
  console.log(`proxy: 0.0.0.0:${PROXY_PORT} -> ${GEMINI_BASE}`);
});

const tg = (method: string, body: object) =>
  fetch(`https://api.telegram.org/bot${TELEGRAM_TOKEN}/${method}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });

const trunc = (s: string, n: number) => (s.length > n ? s.slice(0, n) + "…" : s);

const formatArgs = (args: Record<string, any> | undefined) =>
  !args ? "" : Object.entries(args)
    .map(([k, v]) => `${k}=${typeof v === "string" ? trunc(v, 60) : JSON.stringify(v)}`)
    .join(", ");

async function sendEvent(chat_id: string, event: { type: string; data: any }) {
  const { type, data } = event;
  let text: string | undefined;
  switch (type) {
    case "tool.call":
      text = `🔧 ${data.name}(${formatArgs(data.args)})`;
      break;
    case "tool.result":
      text = `↩️ ${trunc(String(data.result ?? "").trim() || "(empty)", 1000)}`;
      break;
    case "llm.response":
      if (data.content?.trim()) text = data.content.trim();
      break;
    case "llm.error":
      text = `❌ ${data.error}`;
      break;
    case "error":
      text = `❌ ${data.message}`;
      break;
  }
  if (text) await tg("sendMessage", { chat_id, text: trunc(text, 4000) });
}

async function handle({ message }: Update) {
  if (!message?.text) return;
  const chat_id = String(message.chat.id);
  const userDir = join(DATA, chat_id);
  mkdirSync(userDir, { recursive: true });
  try { chownSync(userDir, 1000, 1000); } catch {}
  console.log(`[${chat_id}] ${message.text}`);

  const proc = spawn("docker", [
    "run", "--rm",
    "--add-host=host.docker.internal:host-gateway",
    "-e", `SHOT_CONFIG_GEMINI_LLM_URL=http://host.docker.internal:${PROXY_PORT}`,
    "-e", "SHOT_CONFIG_GEMINI_API_KEY=via-proxy",
    "-v", `${userDir}:/home/agent/.local/share/shot`,
    "--memory", MEMORY, "--cpus", CPUS,
    IMAGE, "--json", "--tools", `--session=${chat_id}`, message.text,
  ], { stdio: ["ignore", "pipe", "pipe"] });

  const killer = setTimeout(() => proc.kill("SIGKILL"), 120_000);
  let sawEvents = false;
  try {
    const rl = createInterface({ input: proc.stdout });
    for await (const line of rl) {
      if (!line.trim()) continue;
      try {
        const event = JSON.parse(line);
        sawEvents = true;
        await sendEvent(chat_id, event);
      } catch {}
    }
    const code: number | null = await new Promise((r) => proc.on("close", r));
    if (code !== 0 && !sawEvents) {
      await tg("sendMessage", { chat_id, text: "error processing your message" });
    }
  } catch (e: any) {
    console.error(`[${chat_id}]`, e.message);
    if (!sawEvents) await tg("sendMessage", { chat_id, text: "error processing your message" });
  } finally {
    clearTimeout(killer);
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
