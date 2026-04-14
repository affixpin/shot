use clap::{Parser, Subcommand};
use serde::Deserialize;
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "armaments", about = "Event source runner for shot agent")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Armament name (loads from TOML config)
    name: Option<String>,

    /// Override interval in seconds
    #[arg(short, long)]
    interval: Option<u64>,

    /// Run once and exit (don't loop)
    #[arg(long)]
    once: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Run healthchecks on all armaments (or a specific one)
    Check {
        /// Specific armament to check (defaults to all)
        name: Option<String>,
    },
}

#[derive(Deserialize)]
struct ArmamentSpec {
    name: String,
    command: String,
    healthcheck: String,
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
    /// Hardcoded value in TOML. CLI --vars.X overrides this.
    #[serde(default)]
    value: Option<String>,
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

fn apply_env(cmd: &mut std::process::Command, spec: &ArmamentSpec, vars: &HashMap<String, String>) {
    // Priority: CLI --vars.X > TOML `value` > TOML `default`
    for (name, var) in &spec.vars {
        if let Some(val) = &var.value {
            cmd.env(name, val);
        }
    }
    for (k, v) in vars {
        cmd.env(k, v);
    }
    for (name, var) in &spec.vars {
        if !vars.contains_key(name) && var.value.is_none() {
            if let Some(default) = &var.default {
                cmd.env(name, default);
            }
        }
    }
}

fn run_command(spec: &ArmamentSpec, vars: &HashMap<String, String>) {
    let mut cmd = std::process::Command::new("sh");
    cmd.arg("-c").arg(&spec.command);
    apply_env(&mut cmd, spec, vars);

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

fn run_healthcheck(spec: &ArmamentSpec, vars: &HashMap<String, String>) -> bool {
    let mut hc = std::process::Command::new("sh");
    hc.arg("-c").arg(&spec.healthcheck);
    hc.stdout(std::process::Stdio::null());
    hc.stderr(std::process::Stdio::null());
    apply_env(&mut hc, spec, vars);
    hc.status().map(|s| s.success()).unwrap_or(false)
}

fn validate_required_vars(spec: &ArmamentSpec, vars: &HashMap<String, String>) -> Vec<String> {
    let mut missing = Vec::new();
    for (name, var) in &spec.vars {
        if var.required && !vars.contains_key(name) && var.value.is_none() && var.default.is_none() {
            missing.push(name.clone());
        }
    }
    missing
}

fn check_one(name: &str, vars: &HashMap<String, String>) {
    let spec = match load_spec_opt(name) {
        Some(s) => s,
        None => { println!("\x1b[31m✗\x1b[0m {name}  (not found)"); return; }
    };
    let missing = validate_required_vars(&spec, vars);
    if !missing.is_empty() {
        println!("\x1b[31m✗\x1b[0m {name}  (missing vars: {})", missing.join(", "));
        return;
    }
    if run_healthcheck(&spec, vars) {
        println!("\x1b[32m✓\x1b[0m {name}");
    } else {
        println!("\x1b[31m✗\x1b[0m {name}  ({})", spec.healthcheck);
    }
}

fn load_spec_opt(name: &str) -> Option<ArmamentSpec> {
    let path = armaments_dir().join(format!("{name}.toml"));
    let content = std::fs::read_to_string(&path).ok()?;
    toml::from_str(&content).ok()
}

fn check_all(vars: &HashMap<String, String>) {
    let dir = armaments_dir();
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            if p.extension()? == "toml" {
                Some(p.file_stem()?.to_string_lossy().to_string())
            } else { None }
        })
        .collect();
    names.sort();
    for name in names {
        check_one(&name, vars);
    }
}

fn main() {
    // Pre-parse --vars.X flags before clap sees them
    let mut raw_args: Vec<String> = std::env::args().collect();
    let cli_vars = extract_var_flags(&mut raw_args);

    let cli = Cli::parse_from(raw_args);

    // Check subcommand
    if let Some(Command::Check { name }) = cli.command {
        match name {
            Some(n) => check_one(&n, &cli_vars),
            None => check_all(&cli_vars),
        }
        return;
    }

    // Normal run: name is required
    let name = match cli.name {
        Some(n) => n,
        None => {
            eprintln!("Usage: armaments <name>");
            eprintln!("       armaments check [name]");
            std::process::exit(1);
        }
    };

    let spec = load_spec(&name);

    let missing = validate_required_vars(&spec, &cli_vars);
    if !missing.is_empty() {
        eprintln!("Error: missing required vars: {}", missing.join(", "));
        eprintln!("Set with: armaments {name} --vars.NAME=value");
        std::process::exit(1);
    }

    if !run_healthcheck(&spec, &cli_vars) {
        eprintln!("Armament '{name}' healthcheck failed: {}", spec.healthcheck);
        std::process::exit(1);
    }

    let interval = cli.interval.unwrap_or(spec.interval);

    if cli.once {
        run_command(&spec, &cli_vars);
        return;
    }

    loop {
        run_command(&spec, &cli_vars);
        std::thread::sleep(std::time::Duration::from_secs(interval));
    }
}
