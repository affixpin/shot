use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ── Deserialization types ──────────────────────────────────────────────

#[derive(Deserialize)]
struct ConfigFile {
    agent: AgentConfig,
    #[serde(flatten)]
    providers: HashMap<String, toml::Value>,
}

#[derive(Deserialize)]
struct AgentConfig {
    provider: String,
    #[serde(default)]
    soul_file: String,
    #[serde(default)]
    tools_dir: String,
    #[serde(default = "default_max_turns")]
    max_turns: usize,
}

fn default_max_turns() -> usize { 50 }

#[derive(Deserialize, Default)]
struct ProviderConfig {
    #[serde(default)]
    llm_url: String,
    #[serde(default)]
    api_key: String,
    #[serde(default)]
    model: String,
    #[serde(default)]
    reasoning: Option<String>,
}

// ── Public types ───────────────────────────────────────────────────────

pub struct Config {
    pub llm_url: String,
    pub api_key: String,
    pub model: String,
    pub reasoning: Option<String>,
    pub soul_prompt: String,
    pub max_turns: usize,
    pub tools_dir: String,
}

// ── Fallback defaults ──────────────────────────────────────────────────
//
// Used when no config file exists and when the file is missing fields.
// CLI `--config.X.Y=value` overrides layer on top of this.
const FALLBACK_CONFIG: &str = r#"
[agent]
provider = "gemini"
max_turns = 50

[gemini]
llm_url = "https://generativelanguage.googleapis.com/v1beta/openai"
model = "gemini-3-flash-preview"
reasoning = "high"
api_key = ""
"#;

// ── Paths ──────────────────────────────────────────────────────────────

fn home_dir() -> PathBuf {
    std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("."))
}

fn config_path() -> PathBuf {
    home_dir().join(".config/shot/agent.toml")
}

fn data_dir() -> PathBuf {
    home_dir().join(".local/share/shot")
}

fn resolve(base: &Path, path: &str) -> String {
    if path.is_empty() { return String::new(); }
    let p = PathBuf::from(path);
    if p.is_absolute() { path.to_string() } else { base.join(path).to_string_lossy().to_string() }
}

// ── Generic merge helpers ──────────────────────────────────────────────

/// Deep-merge `overlay` into `base`: tables recurse, scalars replace.
fn deep_merge(base: &mut toml::Value, overlay: toml::Value) {
    match (base, overlay) {
        (toml::Value::Table(b), toml::Value::Table(o)) => {
            for (k, v) in o {
                match b.get_mut(&k) {
                    Some(existing) => deep_merge(existing, v),
                    None => { b.insert(k, v); }
                }
            }
        }
        (slot, o) => *slot = o,
    }
}

/// Set a value by dotted path, creating intermediate tables as needed.
fn set_path(root: &mut toml::Value, path: &[String], val: toml::Value) {
    let Some(table) = root.as_table_mut() else { return };
    if path.len() == 1 {
        table.insert(path[0].clone(), val);
        return;
    }
    let child = table.entry(path[0].clone())
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
    set_path(child, &path[1..], val);
}

/// Coerce a CLI string into the best-matching TOML scalar.
/// Integers and booleans are parsed; everything else stays a string.
fn parse_scalar(s: &str) -> toml::Value {
    if let Ok(n) = s.parse::<i64>() { return toml::Value::Integer(n); }
    if s == "true"  { return toml::Value::Boolean(true); }
    if s == "false" { return toml::Value::Boolean(false); }
    toml::Value::String(s.to_string())
}

// ── Load ───────────────────────────────────────────────────────────────

impl Config {
    /// Build the final config by layering: fallback defaults → file → CLI overrides.
    ///
    /// `overrides` is a list of (dotted-path, value) pairs from `--config.X.Y=value`.
    /// Validation runs at the end — if `llm_url`, `api_key`, or `model` are empty
    /// for the selected provider, print an actionable error and exit.
    pub fn load(overrides: &[(Vec<String>, String)]) -> Self {
        // 1. Start with fallback defaults.
        let mut merged: toml::Value = toml::from_str(FALLBACK_CONFIG)
            .expect("FALLBACK_CONFIG is malformed — this is a bug");

        // 2. Overlay user's config file, if present.
        let path = config_path();
        if path.exists() {
            let raw = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| die("config", &format!("failed to read {}: {e}", path.display())));
            let file: toml::Value = toml::from_str(&raw)
                .unwrap_or_else(|e| die("config", &format!("failed to parse {}: {e}", path.display())));
            deep_merge(&mut merged, file);
        }

        // 3. Overlay CLI overrides (highest priority).
        for (p, v) in overrides {
            set_path(&mut merged, p, parse_scalar(v));
        }

        // 4. Deserialize into structured form.
        let file: ConfigFile = merged.try_into()
            .unwrap_or_else(|e| die("config", &format!("{e}")));

        let provider_name = file.agent.provider.clone();
        let provider: ProviderConfig = file.providers.get(&provider_name)
            .cloned()
            .and_then(|v| v.try_into().ok())
            .unwrap_or_default();

        // 5. Validate required provider fields.
        let mut missing = Vec::new();
        if provider.llm_url.is_empty() { missing.push(format!("{provider_name}.llm_url")); }
        if provider.api_key.is_empty() { missing.push(format!("{provider_name}.api_key")); }
        if provider.model.is_empty()   { missing.push(format!("{provider_name}.model")); }
        if !missing.is_empty() {
            die_missing(&provider_name, &missing);
        }

        // 6. Resolve paths & read soul.
        let data_dir = data_dir();
        let soul_file = if file.agent.soul_file.is_empty() {
            data_dir.join("SOUL.md").to_string_lossy().to_string()
        } else {
            resolve(&data_dir, &file.agent.soul_file)
        };
        let tools_dir = if file.agent.tools_dir.is_empty() {
            data_dir.join("tools").to_string_lossy().to_string()
        } else {
            resolve(&data_dir, &file.agent.tools_dir)
        };

        Self {
            llm_url: provider.llm_url,
            api_key: provider.api_key,
            model: provider.model,
            reasoning: provider.reasoning,
            soul_prompt: std::fs::read_to_string(&soul_file).unwrap_or_default(),
            max_turns: file.agent.max_turns,
            tools_dir,
        }
    }
}

// ── Error helpers ──────────────────────────────────────────────────────

fn die(title: &str, msg: &str) -> ! {
    eprintln!("{title}: {msg}");
    std::process::exit(1);
}

fn die_missing(provider: &str, missing: &[String]) -> ! {
    eprintln!("Config incomplete. Missing or empty:");
    for m in missing {
        eprintln!("  - {m}");
    }
    eprintln!();
    eprintln!("Fix by one of:");
    eprintln!("  shot --config.{provider}.api_key=<key> \"...\"");
    eprintln!("  shot configure {provider} --api-key <key>");
    if provider == "gemini" {
        eprintln!();
        eprintln!("Get a key: https://aistudio.google.com/apikey");
    }
    std::process::exit(1);
}
