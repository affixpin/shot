use clap::{Parser, Subcommand};
use shotclaw::run::RunOptions;
use std::collections::HashMap;
use std::io::{self, BufRead, IsTerminal, Read};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "shot", about = "Agentic AI assistant")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Session key for persistent conversation (default: current directory path)
    #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = "")]
    session: Option<String>,

    /// Append additional instructions to the system prompt
    #[arg(long)]
    prompt: Option<String>,

    /// Append instructions from a file to the system prompt
    #[arg(long)]
    prompt_file: Option<String>,

    /// Replace the soul (base personality) — overrides SOUL.md
    #[arg(long)]
    soul: Option<String>,

    /// Replace the soul with content from a file
    #[arg(long)]
    soul_file: Option<String>,

    /// Message / scope instruction
    message: Vec<String>,

    /// Quiet mode (no status output, just result)
    #[arg(long)]
    quiet: bool,

    /// JSON output (structured events to stdout, one per line)
    #[arg(long)]
    json: bool,

    /// Pipe mode: read stdin line by line, process each
    #[arg(long)]
    pipe: bool,

    /// Enable all available tools
    #[arg(long = "tools")]
    all_tools: bool,

}

#[derive(Subcommand)]
enum Command {
    /// Set up shot with a provider
    #[command(
        long_about = "\
Set up shot with a provider.

Writes agent.toml to ~/.config/shot/, and installs SOUL.md plus default
tool definitions into ~/.local/share/shot/. Safe to re-run: the config
is overwritten, but existing SOUL.md and tool files are preserved so
your customizations survive.

Supported providers:
  gemini   Google Gemini (gemini-3-flash-preview)

Examples:
  shot configure gemini --api-key AIza...

After configuring, try:
  shot \"hello\"          # one-shot prompt
  shot tools             # list available tools and their healthcheck
  shot sessions list     # inspect stored sessions",
        after_help = "\
Get an API key:
  gemini  https://aistudio.google.com/apikey

Files written:
  ~/.config/shot/agent.toml          main config (provider, model, key)
  ~/.local/share/shot/SOUL.md        base personality (edit freely)
  ~/.local/share/shot/tools/*.toml   tool definitions (edit freely)

Notes:
  - Your API key is stored in plaintext in agent.toml. Keep that file
    readable only by your user (chmod 600 recommended).
  - Re-running `shot configure` overwrites agent.toml but will not
    touch your SOUL.md or tool definitions.
  - To switch providers later, just run `shot configure` again with a
    different provider name and key."
    )]
    Configure {
        /// Provider name (currently supported: gemini)
        provider: String,
        /// API key for the provider — stored in ~/.config/shot/agent.toml
        #[arg(long)]
        api_key: String,
    },
    /// List tools with their healthcheck status
    Tools,
    /// Manage sessions (list / reset)
    Sessions {
        #[command(subcommand)]
        action: Option<SessionAction>,
    },
}

#[derive(Subcommand)]
enum SessionAction {
    /// List all sessions with their sizes and message counts
    List,
    /// Delete a session
    Reset { session: String },
}

fn sessions_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".local/share/shot/sessions")
}

fn list_sessions() {
    let dir = sessions_dir();
    if !dir.exists() {
        eprintln!("No sessions found at {}", dir.display());
        return;
    }

    let sessions = shotclaw::session::Session::list(&dir);
    if sessions.is_empty() {
        eprintln!("No sessions found");
        return;
    }

    for s in sessions {
        let size_str = if s.size_bytes >= 1024 * 1024 {
            format!("{:.1}M", s.size_bytes as f64 / (1024.0 * 1024.0))
        } else if s.size_bytes >= 1024 {
            format!("{:.1}K", s.size_bytes as f64 / 1024.0)
        } else {
            format!("{}B", s.size_bytes)
        };
        println!("{size_str:>7}  {:>4} msgs  {}", s.message_count, s.name);
    }
}

fn resolve_session_key(key: &str) -> String {
    if key.is_empty() {
        std::env::current_dir()
            .ok()
            .map(|p| p.to_string_lossy().replace('/', "_"))
            .unwrap_or_else(|| "default".into())
    } else {
        key.to_string()
    }
}

fn print_result(result: &str, quiet: bool, json: bool) {
    // In JSON mode, the result is already in the event stream — don't print separately
    if json { return; }
    if quiet || !io::stdout().is_terminal() {
        println!("{result}");
    } else {
        termimad::print_text(result);
    }
}

struct ToolFlags {
    enabled: Vec<String>,
    vars: HashMap<String, HashMap<String, String>>,
    metas: HashMap<String, HashMap<String, String>>,
}

/// Pre-parse `--tools.X*` flags from args. Removes them from the args vector.
///
/// Supported forms:
///   --tools.NAME                              → enable tool
///   --tools.NAME.vars.VAR=value               → set var on tool
///   --tools.NAME.META=value                   → set meta-property (e.g. require=true)
fn extract_tool_flags(args: &mut Vec<String>) -> ToolFlags {
    let mut enabled = Vec::new();
    let mut vars: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut metas: HashMap<String, HashMap<String, String>> = HashMap::new();

    let mut i = 0;
    while i < args.len() {
        let arg = args[i].clone();
        let Some(rest) = arg.strip_prefix("--tools.") else { i += 1; continue; };

        // Bare enable: --tools.NAME
        if !rest.contains('=') && !rest.contains('.') {
            let tool_name = rest.to_string();
            if !enabled.contains(&tool_name) { enabled.push(tool_name); }
            args.remove(i);
            continue;
        }

        // Has '=' — inline value
        if let Some(eq_pos) = rest.find('=') {
            let key_part = &rest[..eq_pos];
            let value = rest[eq_pos + 1..].to_string();
            let parts: Vec<&str> = key_part.split('.').collect();
            match parts.as_slice() {
                // --tools.NAME.vars.VAR=value
                [tool, "vars", var] => {
                    vars.entry(tool.to_string()).or_default().insert(var.to_string(), value);
                    args.remove(i);
                    continue;
                }
                // --tools.NAME.META=value
                [tool, meta] => {
                    metas.entry(tool.to_string()).or_default().insert(meta.to_string(), value);
                    args.remove(i);
                    continue;
                }
                _ => {}
            }
        }

        i += 1;
    }

    ToolFlags { enabled, vars, metas }
}

/// Pre-parse `--config.X.Y.Z=value` flags. Removes them from `args`.
/// Returns a list of (dotted-path, value) pairs to overlay onto the config tree.
fn extract_config_flags(args: &mut Vec<String>) -> Vec<(Vec<String>, String)> {
    let mut overrides = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].clone();
        let Some(rest) = arg.strip_prefix("--config.") else { i += 1; continue; };
        let Some(eq) = rest.find('=') else { i += 1; continue; };
        let path: Vec<String> = rest[..eq].split('.').map(String::from).collect();
        let value = rest[eq + 1..].to_string();
        overrides.push((path, value));
        args.remove(i);
    }
    overrides
}

#[tokio::main]
async fn main() {
    // Pre-parse --tools.X and --config.X flags before clap sees them
    let mut raw_args: Vec<String> = std::env::args().collect();
    let config_overrides = extract_config_flags(&mut raw_args);
    let ToolFlags { enabled: mut enabled_tools_list, vars: tool_overrides, metas: tool_metas } =
        extract_tool_flags(&mut raw_args);

    let cli = Cli::try_parse_from(&raw_args).unwrap_or_else(|e| {
        e.print().ok();
        if raw_args.iter().any(|a| a == "configure") {
            eprintln!("\nSupported providers:");
            for (name, desc) in shotclaw::setup::SUPPORTED_PROVIDERS {
                eprintln!("  {name:10} {desc}");
            }
            eprintln!("\nExample:");
            eprintln!("  shot configure gemini --api-key AIza...");
        }
        std::process::exit(e.exit_code());
    });

    // Tools mentioned via vars or metas are also implicitly enabled
    for tool in tool_overrides.keys().chain(tool_metas.keys()) {
        if !enabled_tools_list.contains(tool) {
            enabled_tools_list.push(tool.clone());
        }
    }

    // Required tools = those with meta `require=true`
    let required_tools: Vec<String> = tool_metas.iter()
        .filter(|(_, m)| m.get("require").map(|v| v == "true").unwrap_or(false))
        .map(|(name, _)| name.clone())
        .collect();

    // --tools = all tools, --tools.X = specific tools, neither = no tools
    let enabled_tools = if cli.all_tools {
        None
    } else {
        Some(enabled_tools_list)
    };

    match cli.command {
        Some(Command::Configure { provider, api_key }) => {
            shotclaw::setup::configure(&provider, &api_key);
            return;
        }
        Some(Command::Tools) => {
            let config = shotclaw::Config::load(&config_overrides);
            shotclaw::tools::toolscheck_all(&config.tools_dir, &tool_overrides);
            return;
        }
        Some(Command::Sessions { action }) => {
            match action.unwrap_or(SessionAction::List) {
                SessionAction::List => list_sessions(),
                SessionAction::Reset { session } => {
                    let path = sessions_dir().join(format!("{session}.db"));
                    if path.exists() {
                        std::fs::remove_file(&path).expect("Failed to delete session");
                        eprintln!("Session '{session}' cleared");
                    } else {
                        eprintln!("Session '{session}' not found");
                    }
                }
            }
            return;
        }
        None => {}
    }

    if cli.quiet {
        shotclaw::emit::set_quiet();
    } else if cli.json {
        shotclaw::emit::set_json();
    }

    let arg_msg = cli.message.join(" ");

    // Resolve session path
    let session_path = cli.session.map(|s| {
        let key = resolve_session_key(&s);
        let dir = sessions_dir();
        let _ = std::fs::create_dir_all(&dir);
        dir.join(format!("{key}.db")).to_string_lossy().to_string()
    });

    fn read_or_die(path: &str) -> String {
        std::fs::read_to_string(path).unwrap_or_else(|e| {
            eprintln!("Error reading {path}: {e}");
            std::process::exit(1);
        })
    }

    // Soul override (replaces SOUL.md): --soul wins over --soul-file
    let soul_override = cli.soul
        .or_else(|| cli.soul_file.as_deref().map(read_or_die));

    // Prompt addition (appended to soul): --prompt wins over --prompt-file
    let prompt_addition = cli.prompt
        .or_else(|| cli.prompt_file.as_deref().map(read_or_die));

    let config = shotclaw::Config::load(&config_overrides);

    // Pipe mode: each stdin line is a message. Args not allowed.
    if cli.pipe {
        if !arg_msg.is_empty() {
            eprintln!("Error: --pipe does not accept message arguments");
            eprintln!("Usage: <source> | shot --pipe");
            std::process::exit(1);
        }

        let stdin = io::stdin();
        for line in stdin.lock().lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => break,
            };
            if line.trim().is_empty() { continue; }

            let opts = RunOptions {
                session_path: session_path.as_deref(),
                message: &line,
                enabled_tools: enabled_tools.clone(),
                tool_overrides: tool_overrides.clone(),
                required_tools: required_tools.clone(),
                soul_override: soul_override.clone(),
                prompt_addition: prompt_addition.clone(),
            };

            match shotclaw::run(&config, opts).await {
                Ok(result) => print_result(&result, cli.quiet, cli.json),
                Err(e) => eprintln!("Error: {e}"),
            }
        }
        return;
    }

    // Normal mode: message = args XOR stdin
    let stdin_data = if !io::stdin().is_terminal() {
        let mut buf = String::new();
        io::stdin().read_to_string(&mut buf).unwrap_or_default();
        buf.trim().to_string()
    } else {
        String::new()
    };

    let message = match (arg_msg.is_empty(), stdin_data.is_empty()) {
        (false, true) => arg_msg,
        (true, false) => stdin_data,
        (false, false) => {
            eprintln!("Error: provide a message via args OR stdin, not both");
            std::process::exit(1);
        }
        (true, true) => {
            eprintln!("Error: no message provided");
            eprintln!("Usage: shot \"message\"");
            eprintln!("       echo \"message\" | shot");
            std::process::exit(1);
        }
    };

    let opts = RunOptions {
        session_path: session_path.as_deref(),
        message: &message,
        enabled_tools,
        tool_overrides,
        required_tools,
        soul_override,
        prompt_addition,
    };

    match shotclaw::run(&config, opts).await {
        Ok(result) => print_result(&result, cli.quiet, cli.json),
        Err(e) => {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    }
}
