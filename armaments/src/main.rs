use clap::Parser;
use serde::Deserialize;
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "armaments", about = "Event source runner for shot agent")]
struct Cli {
    /// Armament name (loads from TOML config)
    name: String,

    /// Override interval in seconds
    #[arg(short, long)]
    interval: Option<u64>,

    /// Run once and exit (don't loop)
    #[arg(long)]
    once: bool,
}

#[derive(Deserialize)]
struct ArmamentSpec {
    name: String,
    listen: String,
    #[serde(default = "default_interval")]
    interval: u64,
    #[serde(default)]
    vars: HashMap<String, VarSpec>,
}

#[derive(Deserialize)]
struct VarSpec {
    #[allow(dead_code)]
    #[serde(default)]
    description: String,
    #[serde(default)]
    required: bool,
    #[serde(default)]
    default: Option<String>,
}

fn default_interval() -> u64 { 5 }

fn armaments_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".local/share/shot/armaments")
}

fn load_spec(name: &str) -> ArmamentSpec {
    let dir = armaments_dir();
    let path = dir.join(format!("{name}.toml"));
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| {
            eprintln!("Armament '{name}' not found at {}", path.display());
            eprintln!("Available:");
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for e in entries.flatten() {
                    if let Some(stem) = e.path().file_stem() {
                        eprintln!("  {}", stem.to_string_lossy());
                    }
                }
            }
            std::process::exit(1);
        });
    toml::from_str(&content)
        .unwrap_or_else(|e| {
            eprintln!("Failed to parse {}: {e}", path.display());
            std::process::exit(1);
        })
}

/// Pre-parse `--vars.NAME=value` flags from args.
/// Removes them from the args vector. Returns the var map.
fn extract_var_flags(args: &mut Vec<String>) -> HashMap<String, String> {
    let mut vars = HashMap::new();
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].clone();
        if let Some(rest) = arg.strip_prefix("--vars.") {
            if let Some(eq_pos) = rest.find('=') {
                let key = rest[..eq_pos].to_string();
                let value = rest[eq_pos + 1..].to_string();
                vars.insert(key, value);
                args.remove(i);
                continue;
            } else if i + 1 < args.len() {
                let key = rest.to_string();
                let value = args[i + 1].clone();
                vars.insert(key, value);
                args.remove(i + 1);
                args.remove(i);
                continue;
            }
        }
        i += 1;
    }
    vars
}

fn run_listen(spec: &ArmamentSpec, vars: &HashMap<String, String>) {
    let mut cmd = std::process::Command::new("sh");
    cmd.arg("-c").arg(&spec.listen);

    // Apply vars: CLI overrides first, then defaults from TOML for unset
    for (k, v) in vars {
        cmd.env(k, v);
    }
    for (name, var) in &spec.vars {
        if !vars.contains_key(name) {
            if let Some(default) = &var.default {
                cmd.env(name, default);
            }
        }
    }

    match cmd.output() {
        Ok(output) => {
            let out = std::io::stdout();
            let mut out = out.lock();
            let _ = out.write_all(&output.stdout);
            let _ = out.flush();

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                if !stderr.is_empty() {
                    eprintln!("{}: {}", spec.name, stderr.trim());
                }
            }
        }
        Err(e) => eprintln!("{}: failed to execute: {e}", spec.name),
    }
}

fn main() {
    // Pre-parse --vars.X flags before clap sees them
    let mut raw_args: Vec<String> = std::env::args().collect();
    let cli_vars = extract_var_flags(&mut raw_args);

    let cli = Cli::parse_from(raw_args);
    let spec = load_spec(&cli.name);

    // Validate required vars
    let mut missing = Vec::new();
    for (name, var) in &spec.vars {
        if var.required && !cli_vars.contains_key(name) && var.default.is_none() {
            missing.push(name.clone());
        }
    }
    if !missing.is_empty() {
        eprintln!("Error: missing required vars: {}", missing.join(", "));
        eprintln!("Set with: armaments {} --vars.NAME=value", cli.name);
        std::process::exit(1);
    }

    let interval = cli.interval.unwrap_or(spec.interval);

    if cli.once {
        run_listen(&spec, &cli_vars);
        return;
    }

    loop {
        run_listen(&spec, &cli_vars);
        std::thread::sleep(std::time::Duration::from_secs(interval));
    }
}
