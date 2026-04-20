import { spawn } from "node:child_process";
import { mkdirSync, chownSync } from "node:fs";
import { join } from "node:path";
import { createServer, IncomingMessage, ServerResponse } from "node:http";
import { createInterface } from "node:readline";

const { TELEGRAM_TOKEN, GEMINI_API_KEY, JINA_API_KEY } = process.env;
if (!TELEGRAM_TOKEN || !GEMINI_API_KEY) {
  console.error("set TELEGRAM_TOKEN and GEMINI_API_KEY (JINA_API_KEY optional)");
  process.exit(1);
}

// Host-side path for per-chat data. When running inside the gateway
// container, this should match the host mount so shot containers
// spawned via the mounted docker socket can mount the same path.
const DATA = process.env.DATA_DIR ?? join(process.cwd(), "user_data");
const TEMPLATE_HOST = process.env.SHOT_TEMPLATE_DIR ?? "/opt/shot-template";
const IMAGE = process.env.SHOT_IMAGE ?? "affixpin/shot:latest";
const MEMORY = process.env.SHOT_MEMORY ?? "128m";
const CPUS = process.env.SHOT_CPUS ?? "0.5";
const PROXY_PORT = Number(process.env.PROXY_PORT ?? 3000);

mkdirSync(DATA, { recursive: true });

type Update = {
  update_id: number;
  message?: {
    text?: string;
    message_id: number;
    chat: { id: number; type: "private" | "group" | "supergroup" | "channel" };
    reply_to_message?: { from?: { id: number } };
  };
};

// Learn our own identity at boot so we can detect mentions + replies-to-bot.
const me = (await (await fetch(
  `https://api.telegram.org/bot${TELEGRAM_TOKEN}/getMe`,
)).json()) as { ok: boolean; result: { id: number; username: string } };
const BOT_ID = me.result.id;
const BOT_HANDLE = "@" + me.result.username;
console.log(`bot handle: ${BOT_HANDLE} (id: ${BOT_ID})`);

// ── Proxy ──────────────────────────────────────────────────────────────
// Single HTTP server, path-routed. Shot containers hit us instead of the
// upstreams directly, so no provider key ever enters a container.
//
//   /gemini/*       → https://generativelanguage.googleapis.com/v1beta/openai/*
//   /jina/search/*  → https://s.jina.ai/*   (strips the prefix)
//   /jina/read/*    → https://r.jina.ai/*   (strips the prefix)

async function forward(
  req: IncomingMessage,
  res: ServerResponse,
  target: string,
  authKey: string,
) {
  const body: Buffer[] = [];
  if (req.method !== "GET" && req.method !== "HEAD") {
    for await (const chunk of req) body.push(chunk);
  }
  const upstream = await fetch(target, {
    method: req.method,
    headers: {
      accept: String(req.headers.accept ?? "application/json"),
      "content-type": String(req.headers["content-type"] ?? "application/json"),
      authorization: `Bearer ${authKey}`,
    },
    body: body.length ? Buffer.concat(body) : undefined,
  });
  res.writeHead(upstream.status, {
    "content-type": upstream.headers.get("content-type") ?? "text/plain",
  });
  if (upstream.body) {
    for await (const chunk of upstream.body) res.write(chunk);
  }
  res.end();
}

createServer(async (req, res) => {
  try {
    const url = req.url ?? "";
    if (url.startsWith("/gemini/")) {
      await forward(
        req,
        res,
        `https://generativelanguage.googleapis.com/v1beta/openai${url.slice(7)}`,
        GEMINI_API_KEY!,
      );
      return;
    }
    if (url.startsWith("/jina/search")) {
      if (!JINA_API_KEY) { res.writeHead(503).end(); return; }
      await forward(req, res, `https://s.jina.ai${url.slice("/jina/search".length)}`, JINA_API_KEY);
      return;
    }
    if (url.startsWith("/jina/read/")) {
      if (!JINA_API_KEY) { res.writeHead(503).end(); return; }
      await forward(req, res, `https://r.jina.ai/${url.slice("/jina/read/".length)}`, JINA_API_KEY);
      return;
    }
    res.writeHead(404).end();
  } catch (e: any) {
    console.error("proxy:", e.message);
    if (!res.headersSent) res.writeHead(502);
    res.end();
  }
}).listen(PROXY_PORT, "0.0.0.0", () => {
  console.log(`proxy: 0.0.0.0:${PROXY_PORT}`);
});

// ── Telegram helpers ───────────────────────────────────────────────────

const tg = (method: string, body: object) =>
  fetch(`https://api.telegram.org/bot${TELEGRAM_TOKEN}/${method}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });

const trunc = (s: string, n: number) => (s.length > n ? s.slice(0, n) + "…" : s);

const htmlEscape = (s: string) =>
  s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

// Minimal GFM→Telegram-HTML conversion. Covers the pieces Gemini commonly
// emits: fenced code blocks, inline code, bold. Everything else falls
// through as plain text (Telegram HTML doesn't support headings/lists anyway).
function mdToHtml(md: string): string {
  let s = htmlEscape(md);
  // Fenced code blocks first so their contents don't get re-processed.
  s = s.replace(/```(\w*)\n?([\s\S]*?)```/g, (_m, _lang, code) => `<pre>${code}</pre>`);
  // Inline code
  s = s.replace(/`([^`\n]+)`/g, "<code>$1</code>");
  // Bold **x**
  s = s.replace(/\*\*([^*\n]+)\*\*/g, "<b>$1</b>");
  return s;
}

const formatArgs = (args: Record<string, any> | undefined) =>
  !args ? "" : Object.entries(args)
    .map(([k, v]) => `${k}=${typeof v === "string" ? trunc(v, 60) : JSON.stringify(v)}`)
    .join(", ");

async function sendEvent(
  chat_id: string,
  reply_to: number | undefined,
  event: { type: string; data: any },
) {
  const { type, data } = event;
  const base = { chat_id, reply_to_message_id: reply_to };
  switch (type) {
    case "tool.call":
      await tg("sendMessage", { ...base, text: `🔧 ${data.name}(${formatArgs(data.args)})` });
      break;
    case "tool.result": {
      const result = String(data.result ?? "").trim() || "(empty)";
      await tg("sendMessage", {
        ...base,
        text: `↩️\n<pre>${htmlEscape(trunc(result, 3500))}</pre>`,
        parse_mode: "HTML",
      });
      break;
    }
    case "llm.response":
      if (data.content?.trim()) {
        await tg("sendMessage", {
          ...base,
          text: trunc(mdToHtml(data.content.trim()), 4000),
          parse_mode: "HTML",
        });
      }
      break;
    case "llm.error":
      await tg("sendMessage", { ...base, text: `❌ ${data.error}` });
      break;
    case "error":
      await tg("sendMessage", { ...base, text: `❌ ${data.message}` });
      break;
  }
}

// ── Shot invocation ────────────────────────────────────────────────────

async function handle({ message }: Update) {
  if (!message?.text) return;

  // In groups/supergroups, only respond when @mentioned OR when the user
  // is replying to one of the bot's own messages. Strip the handle so shot
  // doesn't see it in the message text.
  let text = message.text;
  const isGroup = message.chat.type !== "private";
  if (isGroup) {
    const isMention = text.includes(BOT_HANDLE);
    const isReplyToBot = message.reply_to_message?.from?.id === BOT_ID;
    if (!isMention && !isReplyToBot) return;
    text = text.replaceAll(BOT_HANDLE, "").trim();
    if (!text) return;
  }

  const chat_id = String(message.chat.id);
  const message_id = message.message_id;
  const userDir = join(DATA, chat_id);
  mkdirSync(userDir, { recursive: true });
  try { chownSync(userDir, 1000, 1000); } catch {}
  console.log(`[${chat_id}${isGroup ? " group" : ""}] ${text}`);

  const args = [
    "run", "--rm",
    "--add-host=host.docker.internal:host-gateway",
    "-e", "SHOT_CONFIG_AGENT_PROVIDER=gateway",
    "-e", `SHOT_CONFIG_GATEWAY_LLM_URL=http://host.docker.internal:${PROXY_PORT}/gemini`,
    "-e", "SHOT_CONFIG_AGENT_TOOLS_DIR=/srv/shot-template/tools",
    "-e", "SHOT_CONFIG_AGENT_SOUL_FILE=/srv/shot-template/SOUL.md",
    "-e", "SHOT_CONFIG_AGENT_SKILLS_DIR=/srv/shot-template/skills",
    "-v", `${TEMPLATE_HOST}:/srv/shot-template:ro`,
    "-v", `${userDir}:/home/agent/.local/share/shot`,
    "--memory", MEMORY, "--cpus", CPUS,
    IMAGE, "--json", "--tools", `--session=${chat_id}`,
  ];
  if (isGroup) args.push("--skills.project_manager");
  args.push(text);

  const proc = spawn("docker", args, { stdio: ["ignore", "pipe", "pipe"] });

  const killer = setTimeout(() => proc.kill("SIGKILL"), 120_000);
  const replyTo = isGroup ? message_id : undefined;
  let sawEvents = false;
  try {
    const rl = createInterface({ input: proc.stdout });
    for await (const line of rl) {
      if (!line.trim()) continue;
      try {
        const event = JSON.parse(line);
        sawEvents = true;
        await sendEvent(chat_id, replyTo, event);
      } catch {}
    }
    const code: number | null = await new Promise((r) => proc.on("close", r));
    if (code !== 0 && !sawEvents) {
      await tg("sendMessage", { chat_id, text: "error processing your message", reply_to_message_id: replyTo });
    }
  } catch (e: any) {
    console.error(`[${chat_id}]`, e.message);
    if (!sawEvents) await tg("sendMessage", { chat_id, text: "error processing your message", reply_to_message_id: replyTo });
  } finally {
    clearTimeout(killer);
  }
}

// ── Polling loop ───────────────────────────────────────────────────────

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
      console.log("update:", JSON.stringify(u).slice(0, 500));
      handle(u).catch(console.error);
    }
  } catch (e: any) {
    console.error("poll:", e.message);
    await new Promise((r) => setTimeout(r, 5000));
  }
}
