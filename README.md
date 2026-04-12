<div align="center">
  <img src="shooter.jpeg" width="200" />
  <h1>shot agent</h1>
  <p><strong>minimal, fast, portable, granular unix-friendly general purpose agent</strong></p>
</div>

---

## why

I built shot because I don't like bloated software — and that's what most existing AI agents are.

They ship as massive frameworks with bundled runtimes, plugin systems, and thousands of dependencies. They aren't designed to be combined with other Unix tools. They consume a lot of resources. They aren't designed to be easily integrable into workflows or infrastructure.

Shot exists to be different:

- **Minimal** — ~1200 lines of Rust total. Single binary, fast startup, low memory. Can be used for multi-tenant agent setups where you need dozens of agents running without burning resources.
- **Granular** — tools are plain TOML files that shell out to any command. Permissions are controlled by which tools a role has access to. No code changes needed to add, remove, or restrict capabilities.
- **Unix-first** — stdin in, stdout out. Pipe it, redirect it, loop it, compose it. `shot` is a filter, not a framework. It works with `grep`, `jq`, `curl`, `tee`, `>>` and everything else in your terminal.
- **Portable** — one static binary, one config file, a directory of TOML tools. Copy it anywhere and it works.

I love workflows built from minimal, composable tools — `vim`, `fzf`, `ripgrep`, `jq` — tools that do one thing well and combine through pipes. Shot is built with the same mindset.
