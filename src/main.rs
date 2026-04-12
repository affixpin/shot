use clap::{Parser, Subcommand};
use std::io::{self, IsTerminal, Read};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "shot", about = "Agentic AI assistant")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Role to use (e.g. brain)
    #[arg(short, long)]
    role: Option<String>,

    /// Session key for persistent conversation (default: current directory path)
    #[arg(short, long, num_args = 0..=1, require_equals = true, default_missing_value = "")]
    session: Option<String>,

    /// Message / scope instruction
    message: Vec<String>,

    /// Quiet mode (no status output, just result)
    #[arg(short, long)]
    quiet: bool,

    /// Verbose output (JSON events to stdout)
    #[arg(short, long)]
    verbose: bool,

    /// Debug output (detailed events to stderr)
    #[arg(short, long)]
    debug: bool,

    /// Show full tool output (no truncation)
    #[arg(short, long)]
    full: bool,
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
    /// Check which tools are available
    Healthcheck,
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

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    match cli.command {
        Some(Command::Configure { provider, api_key }) => {
            shotclaw::setup::configure(&provider, &api_key);
            return;
        }
        Some(Command::Healthcheck) => {
            let config = shotclaw::Config::load();
            shotclaw::tools::healthcheck_all(&config.tools_dir);
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
    } else if cli.verbose {
        shotclaw::emit::set_verbose();
    } else if cli.debug {
        shotclaw::emit::set_debug();
    }
    if cli.full {
        shotclaw::emit::set_full();
    }

    let arg_msg = cli.message.join(" ");

    let stdin_data = if !io::stdin().is_terminal() {
        let mut buf = String::new();
        io::stdin().read_to_string(&mut buf).unwrap_or_default();
        buf.trim().to_string()
    } else {
        String::new()
    };

    // Resolve session path
    let session_path = cli.session.map(|s| {
        let key = resolve_session_key(&s);
        let dir = sessions_dir();
        let _ = std::fs::create_dir_all(&dir);
        dir.join(format!("{key}.db")).to_string_lossy().to_string()
    });

    // stdin = context, args = message
    let (context, message) = match (arg_msg.is_empty(), stdin_data.is_empty()) {
        (false, false) => (stdin_data, arg_msg),
        (false, true) => (String::new(), arg_msg),
        (true, false) => (String::new(), stdin_data),
        (true, true) => {
            eprintln!("Error: no message provided");
            eprintln!("Usage: shot \"message\"");
            eprintln!("       shot -s \"message\"");
            eprintln!("       echo \"context\" | shot \"instruction\"");
            std::process::exit(1);
        }
    };

    let config = shotclaw::Config::load();

    match shotclaw::run(&config, cli.role.as_deref(), session_path.as_deref(), &context, &message).await {
        Ok(result) => {
            if cli.quiet || cli.verbose || !std::io::IsTerminal::is_terminal(&std::io::stdout()) {
                println!("{result}");
            } else {
                termimad::print_text(&result);
            }
        }
        Err(e) => {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    }
}
