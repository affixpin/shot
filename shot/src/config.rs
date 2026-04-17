use crate::setup;
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
// Env vars layer on top, then CLI `--config.X.Y=value` flags (highest priority).
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

// ── Public: merged config tree ─────────────────────────────────────────

/// Build the fully-merged config tree without deserializing or validating.
/// Layers, lowest to highest priority:
///   1. FALLBACK_CONFIG (compiled-in defaults)
///   2. ~/.config/shot/agent.toml, if present
///   3. Env vars for known providers: `<NAME>_API_KEY`
///   4. CLI `--config.X.Y=value` overrides
///
/// Used by `Config::load` and by the `shot config show` subcommand.
pub fn merged_toml(overrides: &[(Vec<String>, String)]) -> toml::Value {
    let mut merged: toml::Value = toml::from_str(FALLBACK_CONFIG)
        .expect("FALLBACK_CONFIG is malformed — this is a bug");

    // Layer 2: user's config file.
    let path = config_path();
    if path.exists() {
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| die("config", &format!("failed to read {}: {e}", path.display())));
        let file: toml::Value = toml::from_str(&raw)
            .unwrap_or_else(|e| die("config", &format!("failed to parse {}: {e}", path.display())));
        deep_merge(&mut merged, file);
    }

    // Layer 3a: convenience env vars for common case.
    // `<PROVIDER>_API_KEY` (e.g. GEMINI_API_KEY) → `[provider] api_key = ...`.
    for (name, _desc) in setup::SUPPORTED_PROVIDERS {
        let var = format!("{}_API_KEY", name.to_uppercase());
        if let Ok(key) = std::env::var(&var) {
            if !key.is_empty() {
                set_path(
                    &mut merged,
                    &[name.to_string(), "api_key".to_string()],
                    toml::Value::String(key),
                );
            }
        }
    }

    // Layer 3b: generic `SHOT_CONFIG_<SECTION>_<FIELD>=value` env vars,
    // mirroring the `--config.<section>.<field>=value` CLI flag 1:1.
    // First underscore after the prefix splits section from field; rest
    // of the underscores are preserved in the field name (so multi-word
    // fields like `max_turns` and `api_key` round-trip correctly).
    // Takes precedence over the convenience layer above.
    for (name, val) in std::env::vars() {
        let Some(rest) = name.strip_prefix("SHOT_CONFIG_") else { continue; };
        if val.is_empty() { continue; }
        let Some(sep) = rest.find('_') else { continue; };
        let section = rest[..sep].to_lowercase();
        let field = rest[sep + 1..].to_lowercase();
        set_path(&mut merged, &[section, field], parse_scalar(&val));
    }

    // Layer 4: CLI overrides (highest priority).
    for (p, v) in overrides {
        set_path(&mut merged, p, parse_scalar(v));
    }

    merged
}

// ── Auto-bootstrap ─────────────────────────────────────────────────────

/// On first run, extract embedded defaults (tools + SOUL.md) to disk.
/// Skipped entirely if the target directory already exists, so user
/// customizations are never overwritten.
fn bootstrap_defaults(tools_dir: &str, soul_file: &str) {
    let tools_path = Path::new(tools_dir);
    if !tools_path.exists() && std::fs::create_dir_all(tools_path).is_ok() {
        for (name, content) in setup::DEFAULT_TOOLS {
            let _ = std::fs::write(tools_path.join(name), content);
        }
    }

    let soul_path = Path::new(soul_file);
    if !soul_path.exists() {
        if let Some(parent) = soul_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(soul_path, setup::DEFAULT_SOUL);
    }
}

// ── Load ───────────────────────────────────────────────────────────────

impl Config {
    /// Build the final config from layered sources, validate required
    /// fields, auto-bootstrap tools/soul on first run, and return the
    /// derived struct. Exits the process with an actionable error if
    /// the selected provider is missing required fields.
    pub fn load(overrides: &[(Vec<String>, String)]) -> Self {
        let merged = merged_toml(overrides);

        let file: ConfigFile = merged.try_into()
            .unwrap_or_else(|e| die("config", &format!("{e}")));

        let provider_name = file.agent.provider.clone();
        let provider: ProviderConfig = file.providers.get(&provider_name)
            .cloned()
            .and_then(|v| v.try_into().ok())
            .unwrap_or_default();

        // Validate required provider fields.
        let mut missing = Vec::new();
        if provider.llm_url.is_empty() { missing.push(format!("{provider_name}.llm_url")); }
        if provider.api_key.is_empty() { missing.push(format!("{provider_name}.api_key")); }
        if provider.model.is_empty()   { missing.push(format!("{provider_name}.model")); }
        if !missing.is_empty() {
            die_missing(&provider_name, &missing);
        }

        // Resolve paths.
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

        // First-run bootstrap: extract embedded defaults to disk.
        bootstrap_defaults(&tools_dir, &soul_file);

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
    let env_var = format!("{}_API_KEY", provider.to_uppercase());
    eprintln!("Config incomplete. Missing or empty:");
    for m in missing {
        eprintln!("  - {m}");
    }
    eprintln!();
    eprintln!("Fix by any of:");
    eprintln!("  export {env_var}=<key>");
    eprintln!("  shot --config.{provider}.api_key=<key> \"...\"");
    eprintln!("  shot --config.{provider}.api_key=<key> config show > ~/.config/shot/agent.toml");
    if provider == "gemini" {
        eprintln!();
        eprintln!("Get a key: https://aistudio.google.com/apikey");
    }
    std::process::exit(1);
}
