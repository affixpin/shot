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
    #[arg(short, long, num_args = 0..=1, require_equals = true, default_missing_value = "")]
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
    #[arg(short, long)]
    quiet: bool,

    /// JSON output (structured events to stdout, one per line)
    #[arg(short, long)]
    json: bool,

    /// Show full tool output (no truncation)
    #[arg(short, long)]
    full: bool,

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
    Configure {
        provider: String,
        #[arg(long)]
        api_key: String,
    },
    /// Clear a session
    Reset {
        session: String,
    },
    /// List tools with their healthcheck status
    Tools,
}

fn sessions_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".local/share/shot/sessions")
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

/// Pre-parse `--tools.X` and `--tools.X.var=value` flags from args.
/// Removes them from the args vector. Returns (enabled_tools, overrides).
fn extract_tool_flags(args: &mut Vec<String>) -> (Vec<String>, HashMap<String, HashMap<String, String>>) {
    let mut enabled = Vec::new();
    let mut overrides: HashMap<String, HashMap<String, String>> = HashMap::new();

    let mut i = 0;
    while i < args.len() {
        let arg = args[i].clone();
        if let Some(rest) = arg.strip_prefix("--tools.") {
            // Cases:
            //   --tools.NAME                            → enable tool
            //   --tools.NAME.VAR=VALUE                  → set var
            //   --tools.NAME.VAR VALUE  (next arg)      → set var
            if let Some(eq_pos) = rest.find('=') {
                let key_part = &rest[..eq_pos];
                let value = rest[eq_pos + 1..].to_string();
                if let Some(dot_pos) = key_part.find('.') {
                    let tool_name = key_part[..dot_pos].to_string();
                    let var_name = key_part[dot_pos + 1..].to_string();
                    overrides.entry(tool_name).or_default().insert(var_name, value);
                    args.remove(i);
                    continue;
                }
            } else if let Some(dot_pos) = rest.find('.') {
                // --tools.NAME.VAR VALUE (separate arg)
                let tool_name = rest[..dot_pos].to_string();
                let var_name = rest[dot_pos + 1..].to_string();
                if i + 1 < args.len() {
                    let value = args[i + 1].clone();
                    overrides.entry(tool_name).or_default().insert(var_name, value);
                    args.remove(i + 1);
                    args.remove(i);
                    continue;
                }
            } else {
                // --tools.NAME (just enabling)
                let tool_name = rest.to_string();
                if !enabled.contains(&tool_name) { enabled.push(tool_name); }
                args.remove(i);
                continue;
            }
        }
        i += 1;
    }

    (enabled, overrides)
}

#[tokio::main]
async fn main() {
    // Pre-parse --tools.X flags before clap sees them
    let mut raw_args: Vec<String> = std::env::args().collect();
    let (enabled_tools_list, tool_overrides) = extract_tool_flags(&mut raw_args);

    let cli = Cli::parse_from(raw_args);

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
            let config = shotclaw::Config::load();
            shotclaw::tools::toolscheck_all(&config.tools_dir, &tool_overrides);
            return;
        }
        Some(Command::Reset { session }) => {
            let path = sessions_dir().join(format!("{session}.db"));
            if path.exists() {
                std::fs::remove_file(&path).expect("Failed to delete session");
                eprintln!("Session '{session}' cleared");
            } else {
                eprintln!("Session '{session}' not found");
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
    if cli.full {
        shotclaw::emit::set_full();
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

    let config = shotclaw::Config::load();

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
