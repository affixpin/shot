use clap::{Parser, Subcommand};
use std::io::{self, IsTerminal, Read};

#[derive(Parser)]
#[command(name = "shot", about = "Agentic AI assistant")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Message to process
    message: Vec<String>,

    /// Verbose output (JSON events to stdout)
    #[arg(short, long)]
    verbose: bool,

    /// Pretty output (colored events to stderr)
    #[arg(short, long)]
    pretty: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Set up shot with a provider (e.g. shot configure gemini)
    Configure {
        /// Provider name
        provider: String,
        /// API key
        #[arg(long)]
        api_key: String,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    if let Some(Command::Configure { provider, api_key }) = cli.command {
        shotclaw::config::configure(&provider, &api_key);
        return;
    }

    if cli.pretty {
        shotclaw::emit::set_pretty();
    } else if cli.verbose {
        shotclaw::emit::set_verbose();
    }

    let arg_msg = cli.message.join(" ");

    let stdin_data = if !io::stdin().is_terminal() {
        let mut buf = String::new();
        io::stdin().read_to_string(&mut buf).unwrap_or_default();
        buf.trim().to_string()
    } else {
        String::new()
    };

    let msg = match (arg_msg.is_empty(), stdin_data.is_empty()) {
        (false, false) => format!("{arg_msg}\n\n{stdin_data}"),
        (false, true) => arg_msg,
        (true, false) => stdin_data,
        (true, true) => {
            eprintln!("Error: no message provided");
            eprintln!("Usage: shot \"message\" or echo \"data\" | shot \"prompt\"");
            std::process::exit(1);
        }
    };

    let config = shotclaw::Config::load();

    match shotclaw::run(&config, &msg).await {
        Ok(result) => println!("{result}"),
        Err(e) => {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    }
}
