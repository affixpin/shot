import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { mkdirSync, chownSync } from "node:fs";
import { join } from "node:path";
import { createServer } from "node:http";

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
const PROXY_PORT = Number(process.env.PROXY_PORT ?? 3000);
const GEMINI_BASE = "https://generativelanguage.googleapis.com/v1beta/openai";

mkdirSync(DATA, { recursive: true });

type Update = {
  update_id: number;
  message?: { text?: string; chat: { id: number } };
};

// Proxy LLM endpoint. Shot containers talk to this instead of Gemini
// directly, so the real API key never enters a shot container and can't
// be exfiltrated via tool calls.
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
  // Bind on all interfaces so containers reach it via host.docker.internal
  // (which resolves to the Docker bridge gateway, not loopback). GCP's
  // default-deny ingress firewall keeps this port private in practice.
  console.log(`proxy: 0.0.0.0:${PROXY_PORT} -> ${GEMINI_BASE}`);
});

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
  // Shot runs as uid 1000 ('agent') inside the container. Chown the mount
  // so it can write its session file. No-op if we lack privileges (local dev).
  try { chownSync(userDir, 1000, 1000); } catch {}
  console.log(`[${chat_id}] ${message.text}`);
  try {
    const { stdout } = await exec("docker", [
      "run", "--rm",
      "--add-host=host.docker.internal:host-gateway",
      "-e", `SHOT_CONFIG_GEMINI_LLM_URL=http://host.docker.internal:${PROXY_PORT}`,
      "-e", "SHOT_CONFIG_GEMINI_API_KEY=via-proxy",
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
