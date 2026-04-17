<div align="center">
  <img src="shooter.jpeg" width="200" />
  <h1>shot agent</h1>
  <p><strong>minimal, portable unix-friendly agent</strong></p>
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

### telegram bot in one container

Shot ships with a companion binary, `armaments`, that polls event sources and prints each event to stdout. Pipe it into `shot --pipe`.

```bash
docker run --rm -i \
  -e SHOT_CONFIG_GEMINI_API_KEY=YOUR_GEMINI_KEY \
  -e BOT_TOKEN=YOUR_TELEGRAM_BOT_TOKEN \
  affixpin/shot sh -c '
    armaments telegram --vars.bot_token=$BOT_TOKEN \
      | shot --pipe --tools --tools.tg_send.vars.bot_token=$BOT_TOKEN
  '
```

Armaments long-polls Telegram. Each batch of messages gets printed on one line. Shot reads line by line and replies with `tg_send`.

### build from source

Rust workspace, so `rustup` is the only prerequisite.

```bash
git clone https://github.com/affixpin/shot
cd shot

# install both binaries to ~/.cargo/bin
cargo install --path shot
cargo install --path armaments

# or build without installing
cargo build --release --workspace
sudo ln -s "$(pwd)/target/release/shot" /usr/local/bin/shot
sudo ln -s "$(pwd)/target/release/armaments" /usr/local/bin/armaments
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
