<div align="center">
  <img src="shooter.jpeg" width="200" />
  <h1>shot agent</h1>
  <p><strong>minimal, portable unix-friendly agent</strong></p>
  <p>try the live demo: <a href="./gateway">multitenant telegram bot</a> → <a href="https://t.me/autoshot_bot">@autoshot_bot</a></p>
</div>

---

## why

I built shot because I couldn't find a lean agent for multi-tenant use. Every option was either a huge framework with a bundled runtime and plugins, or a tool to build your own huge framework. None of them did the one thing I wanted: start, use tools, save the session, exit.

What shot is:

- Short-lived. You start it per message, it does the work, exits.
- Extensible. Tools are plain `.toml` files that run shell commands. No code to add or remove one.
- Easy to wire up. `--json` gives you one event per line on stdout, pipe it anywhere.
- Small. 4 MB static binary, 31 MB Docker image, around 3 MB RAM while handling a message, a few milliseconds to cold-start.
- Unix-friendly. Reads stdin, writes stdout, plays well with pipes and redirection.

That last point is the real reason it exists. I like tools like `vim`, `fzf`, `fd`, `rg`, `jq`. Each one does a single thing and gets out of the way. Shot tries to be that kind of tool for running an LLM agent.

## quick start

Easiest way to try shot is Docker.

### gemini

```bash
# simple question, no tools
docker run --rm \
  -e SHOT_CONFIG_GEMINI_API_KEY=YOUR_KEY \
  affixpin/shot "what's the capital of France?"

# with the default tools (file I/O, shell, grep, etc.)
docker run --rm \
  -e SHOT_CONFIG_GEMINI_API_KEY=YOUR_KEY \
  affixpin/shot --tools "list the files in /etc and summarize"

# with web search (free Jina key at https://jina.ai/reader)
docker run --rm \
  -e SHOT_CONFIG_GEMINI_API_KEY=YOUR_KEY \
  affixpin/shot --tools \
  --tools.web_search.vars.jina_api_key=YOUR_JINA_KEY \
  "who is Dmytro Pintak?"
```

### anthropic

```bash
docker run --rm \
  -e SHOT_CONFIG_AGENT_PROVIDER=anthropic \
  -e SHOT_CONFIG_ANTHROPIC_API_KEY=YOUR_KEY \
  affixpin/shot --tools "what's in the current directory?"
```

### openai

```bash
docker run --rm \
  -e SHOT_CONFIG_AGENT_PROVIDER=openai \
  -e SHOT_CONFIG_OPENAI_API_KEY=YOUR_KEY \
  affixpin/shot --tools "what's in the current directory?"
```

### build from source

Rust workspace, so `rustup` is the only prerequisite.

```bash
git clone https://github.com/affixpin/shot
cd shot

# install the binary to ~/.cargo/bin
cargo install --path shot

# or build without installing
cargo build --release -p shot
sudo ln -s "$(pwd)/target/release/shot" /usr/local/bin/shot
```

On the first run shot drops its default tools and soul prompt into `~/.local/share/shot/`. No separate configure step, just set the key:

```bash
export SHOT_CONFIG_GEMINI_API_KEY=YOUR_KEY
shot "hello"
```

## configuration

Config is assembled from four layers. Later layers win.

1. Compiled-in defaults.
2. `~/.config/shot/agent.toml` (or `$XDG_CONFIG_HOME/shot/agent.toml`) if present.
3. Env vars: `SHOT_CONFIG_<SECTION>_<FIELD>=value`.
4. CLI flags: `--config.<section>.<field>=value`.

A few examples:

```bash
# switch provider via env
SHOT_CONFIG_AGENT_PROVIDER=openai SHOT_CONFIG_OPENAI_API_KEY=... shot "..."

# override the model on a single run
shot --config.gemini.model=gemini-2.5-pro "explain quickly"

# persist the current setup to a file
shot --config.gemini.api_key=YOUR_KEY config show > ~/.config/shot/agent.toml
```

`shot config show` prints the merged config as TOML. Handy when a flag or env var isn't doing what you expect.

## tools

Tools live in `~/.local/share/shot/tools/` as `.toml` files. Each one describes a shell command and its parameters; the LLM sees the description, calls the tool with arguments, shot runs the command with those arguments set as env vars.

Default set, installed on first run:

| Tool            | What it does                              |
|-----------------|-------------------------------------------|
| `file_read`     | Read a file                               |
| `file_write`    | Write a file                              |
| `file_remove`   | Delete a file or directory                |
| `list_files`    | `ls -la`                                  |
| `search_text`   | `grep -rn` across files                   |
| `shell`         | Run a shell command                       |
| `web_search`    | Web search via Jina (needs `jina_api_key`) |
| `web_read`      | Fetch a URL as markdown via Jina          |
| `tg_send`       | Send a Telegram message                   |
| `memory_store`  | Save a fact via `engram`                  |
| `memory_recall` | Search saved memory via `engram`          |

To add your own, drop another `.toml` in the tools directory. Shot picks it up next run.

## gateway

Shot is one-shot: start, handle a message, exit. For anything longer-running (a Telegram bot, a webhook, a Slack app) you need a host process that decides when to spawn shot and with which args. That's `shot-gateway` — a minimal Node.js server shipped as a separate image.

The live demo at [@autoshot_bot](https://t.me/autoshot_bot) runs on it. It polls Telegram, spawns one shot container per message with per-chat isolation, and proxies LLM + Jina calls so API keys never enter a shot container.

```bash
docker run -d --name shot-gateway --restart=always \
  -e TELEGRAM_TOKEN=... \
  -e GEMINI_API_KEY=... \
  -e JINA_API_KEY=...                                  # optional, enables web_search/web_read
  -v /var/run/docker.sock:/var/run/docker.sock \
  -v /opt/shot-data:/data \
  affixpin/shot-gateway:latest
```

Full setup (GCP deploy scripts, secret manager wiring, proxy explainer) lives under [`./gateway`](./gateway).
