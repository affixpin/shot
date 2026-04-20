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

    /// Enable all skills in the skills directory
    #[arg(long = "skills")]
    all_skills: bool,

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

    /// Use a specific config file (default: $XDG_CONFIG_HOME/shot/agent.toml or ~/.config/shot/agent.toml)
    #[arg(long)]
    config_file: Option<String>,

    /// Attach an image file to the message. Path or URL. May be repeated.
    #[arg(long = "file", value_name = "path|url")]
    files: Vec<String>,
}

/// Detect image mime type from magic bytes.
fn detect_image_mime(bytes: &[u8]) -> &'static str {
    match bytes {
        [0xFF, 0xD8, 0xFF, ..] => "image/jpeg",
        [0x89, 0x50, 0x4E, 0x47, ..] => "image/png",
        [0x47, 0x49, 0x46, ..] => "image/gif",
        b if b.len() >= 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" => "image/webp",
        _ => "application/octet-stream",
    }
}

/// Resolve `--file` value to an `image_url` string: URL → passthrough, path → data URI.
fn resolve_file(spec: &str) -> String {
    use base64::Engine;
    if spec.starts_with("http://") || spec.starts_with("https://") || spec.starts_with("data:") {
        return spec.to_string();
    }
    let bytes = std::fs::read(spec).unwrap_or_else(|e| {
        eprintln!("Error reading --file {spec}: {e}");
        std::process::exit(1);
    });
    let mime = detect_image_mime(&bytes);
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    format!("data:{mime};base64,{b64}")
}

#[derive(Subcommand)]
enum Command {
    /// List tools with their healthcheck status
    Tools,
    /// Manage sessions (list / reset)
    Sessions {
        #[command(subcommand)]
        action: Option<SessionAction>,
    },
    /// Inspect the effective config
    ///
    /// Prints the fully-merged config (defaults + file + env + CLI overrides)
    /// to stdout. Redirect with `>` to write a persistent config file:
    ///
    ///   shot --config.gemini.api_key=AIza... config show > ~/.config/shot/agent.toml
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
}

#[derive(Subcommand)]
enum SessionAction {
    /// List all sessions with their sizes and message counts
    List,
    /// Delete a session
    Reset { session: String },
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Print the fully-merged effective config as TOML
    Show,
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

/// Pre-parse `--skills.NAME` flags. Removes them from `args`.
/// Returns the list of skill names to activate.
fn extract_skill_flags(args: &mut Vec<String>) -> Vec<String> {
    let mut enabled = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].clone();
        let Some(name) = arg.strip_prefix("--skills.") else { i += 1; continue; };
        if !name.is_empty() && !enabled.contains(&name.to_string()) {
            enabled.push(name.to_string());
        }
        args.remove(i);
    }
    enabled
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
    // Pre-parse --tools.X, --skills.X and --config.X flags before clap sees them
    let mut raw_args: Vec<String> = std::env::args().collect();
    let config_overrides = extract_config_flags(&mut raw_args);
    let enabled_skills_list = extract_skill_flags(&mut raw_args);
    let ToolFlags { enabled: mut enabled_tools_list, vars: tool_overrides, metas: tool_metas } =
        extract_tool_flags(&mut raw_args);

    let cli = Cli::try_parse_from(&raw_args).unwrap_or_else(|e| {
        e.print().ok();
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
        Some(Command::Tools) => {
            let config = shotclaw::Config::load(cli.config_file.as_deref(), &config_overrides);
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
        Some(Command::Config { action: ConfigAction::Show }) => {
            let merged = shotclaw::config::merged_toml(cli.config_file.as_deref(), &config_overrides);
            print!("{}", toml::to_string_pretty(&merged).unwrap());
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

    let config = shotclaw::Config::load(cli.config_file.as_deref(), &config_overrides);

    // Resolve skills. `--skills` activates everything in skills_dir;
    // otherwise only the names passed via `--skills.NAME` are loaded.
    let skills: Vec<String> = if cli.all_skills {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(&config.skills_dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().map(|x| x == "md").unwrap_or(false))
            .collect();
        entries.sort();
        entries.into_iter()
            .filter_map(|p| std::fs::read_to_string(&p).ok())
            .collect()
    } else {
        enabled_skills_list.iter().map(|name| {
            let path = PathBuf::from(&config.skills_dir).join(format!("{name}.md"));
            std::fs::read_to_string(&path).unwrap_or_else(|e| {
                eprintln!("Error reading skill '{name}' from {}: {e}", path.display());
                std::process::exit(1);
            })
        }).collect()
    };

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
                skills: skills.clone(),
                attachments: Vec::new(),
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

    let attachments: Vec<String> = cli.files.iter().map(|s| resolve_file(s)).collect();

    let opts = RunOptions {
        session_path: session_path.as_deref(),
        message: &message,
        enabled_tools,
        tool_overrides,
        required_tools,
        soul_override,
        prompt_addition,
        skills,
        attachments,
    };

    match shotclaw::run(&config, opts).await {
        Ok(result) => print_result(&result, cli.quiet, cli.json),
        Err(e) => {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    }
}
