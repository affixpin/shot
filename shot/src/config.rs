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
    #[serde(default)]
    skills_dir: String,
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
    pub skills_dir: String,
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

[openai]
llm_url = "https://api.openai.com/v1"
model = "gpt-4o"
api_key = ""

[anthropic]
llm_url = "https://api.anthropic.com/v1"
model = "claude-opus-4-7"
api_key = ""
"#;

// ── Paths ──────────────────────────────────────────────────────────────

fn home_dir() -> PathBuf {
    std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("."))
}

fn default_config_path() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join("shot/agent.toml");
        }
    }
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
///   2. Config file (`--config-file` if given, else XDG/default path — if it exists)
///   3. SHOT_CONFIG_<SECTION>_<FIELD> env vars
///   4. CLI `--config.X.Y=value` overrides
///
/// If `config_file` is explicitly provided but doesn't exist, exits with an
/// error. If the default path doesn't exist, silently skips that layer.
///
/// Used by `Config::load` and by the `shot config show` subcommand.
pub fn merged_toml(config_file: Option<&str>, overrides: &[(Vec<String>, String)]) -> toml::Value {
    let mut merged: toml::Value = toml::from_str(FALLBACK_CONFIG)
        .expect("FALLBACK_CONFIG is malformed — this is a bug");

    // Layer 2: config file.
    let (path, required) = match config_file {
        Some(p) => (PathBuf::from(p), true),
        None => (default_config_path(), false),
    };
    if path.exists() {
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| die("config", &format!("failed to read {}: {e}", path.display())));
        let file: toml::Value = toml::from_str(&raw)
            .unwrap_or_else(|e| die("config", &format!("failed to parse {}: {e}", path.display())));
        deep_merge(&mut merged, file);
    } else if required {
        die("config", &format!("--config-file {} does not exist", path.display()));
    }

    // Layer 3: `SHOT_CONFIG_<SECTION>_<FIELD>=value` env vars, mirroring
    // the `--config.<section>.<field>=value` CLI flag 1:1. First underscore
    // after the prefix splits section from field; remaining underscores are
    // preserved in the field name (so multi-word fields like `max_turns`
    // and `api_key` round-trip correctly against the TOML schema).
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

    // Provider auto-detection: if the selected provider has no api_key but
    // exactly one other provider does, switch `agent.provider` to that one.
    // Lets users set SHOT_CONFIG_ANTHROPIC_API_KEY alone without also
    // needing SHOT_CONFIG_AGENT_PROVIDER=anthropic. No-op in the ambiguous
    // case (0 or 2+ configured providers); validation surfaces those.
    apply_auto_detect(&mut merged);

    merged
}

fn apply_auto_detect(merged: &mut toml::Value) {
    let current = merged
        .get("agent")
        .and_then(|v| v.get("provider"))
        .and_then(|v| v.as_str())
        .map(String::from);
    let Some(current) = current else { return };

    let has_key = |root: &toml::Value, name: &str| -> bool {
        root.get(name)
            .and_then(|v| v.get("api_key"))
            .and_then(|v| v.as_str())
            .map(|s| !s.is_empty())
            .unwrap_or(false)
    };

    if has_key(merged, &current) { return; }

    let Some(table) = merged.as_table() else { return };
    let candidates: Vec<String> = table.keys()
        .filter(|k| k.as_str() != "agent" && k.as_str() != current)
        .filter(|k| has_key(merged, k))
        .cloned()
        .collect();

    if candidates.len() == 1 {
        if let Some(agent) = merged.get_mut("agent").and_then(|v| v.as_table_mut()) {
            agent.insert("provider".into(), toml::Value::String(candidates[0].clone()));
        }
    }
}

// ── Auto-bootstrap ─────────────────────────────────────────────────────

/// On first run, extract embedded defaults (tools + skills + SOUL.md) to disk.
/// Each target is checked independently; whichever doesn't exist gets
/// populated. Once a directory/file exists, it's never overwritten — user
/// customizations and edits survive upgrades.
fn bootstrap_defaults(tools_dir: &str, soul_file: &str, skills_dir: &str) {
    let tools_path = Path::new(tools_dir);
    if !tools_path.exists() && std::fs::create_dir_all(tools_path).is_ok() {
        for (name, content) in setup::DEFAULT_TOOLS {
            let _ = std::fs::write(tools_path.join(name), content);
        }
    }

    let skills_path = Path::new(skills_dir);
    if !skills_path.exists() && std::fs::create_dir_all(skills_path).is_ok() {
        for (name, content) in setup::DEFAULT_SKILLS {
            let _ = std::fs::write(skills_path.join(name), content);
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
    pub fn load(config_file: Option<&str>, overrides: &[(Vec<String>, String)]) -> Self {
        let merged = merged_toml(config_file, overrides);

        let file: ConfigFile = merged.try_into()
            .unwrap_or_else(|e| die("config", &format!("{e}")));

        let lookup = |name: &str| -> ProviderConfig {
            file.providers.get(name)
                .cloned()
                .and_then(|v| v.try_into().ok())
                .unwrap_or_default()
        };

        let provider_name = file.agent.provider.clone();
        let provider = lookup(&provider_name);

        // Validate required fields. Auto-detect already ran in merged_toml
        // and switched `agent.provider` when unambiguous, so if we're here
        // with an empty api_key it's one of: no provider has any key, OR
        // multiple providers have keys (ambiguous).
        let mut missing = Vec::new();
        if provider.llm_url.is_empty() { missing.push(format!("{provider_name}.llm_url")); }
        if provider.api_key.is_empty() { missing.push(format!("{provider_name}.api_key")); }
        if provider.model.is_empty()   { missing.push(format!("{provider_name}.model")); }
        if !missing.is_empty() {
            let configured: Vec<String> = file.providers.keys()
                .filter(|n| !lookup(n).api_key.is_empty())
                .cloned()
                .collect();
            match configured.len() {
                0 => die_no_provider_configured(),
                n if n >= 2 => die_ambiguous_provider(&configured),
                _ => die_missing(&provider_name, &missing),
            }
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
        let skills_dir = if file.agent.skills_dir.is_empty() {
            data_dir.join("skills").to_string_lossy().to_string()
        } else {
            resolve(&data_dir, &file.agent.skills_dir)
        };

        // First-run bootstrap: extract embedded defaults to disk.
        bootstrap_defaults(&tools_dir, &soul_file, &skills_dir);

        Self {
            llm_url: provider.llm_url,
            api_key: provider.api_key,
            model: provider.model,
            reasoning: provider.reasoning,
            soul_prompt: std::fs::read_to_string(&soul_file).unwrap_or_default(),
            max_turns: file.agent.max_turns,
            tools_dir,
            skills_dir,
        }
    }
}

// ── Error helpers ──────────────────────────────────────────────────────

fn die(title: &str, msg: &str) -> ! {
    eprintln!("{title}: {msg}");
    std::process::exit(1);
}

fn die_missing(provider: &str, missing: &[String]) -> ! {
    let section_upper = provider.to_uppercase();
    eprintln!("Config incomplete for provider '{provider}'. Missing or empty:");
    for m in missing {
        eprintln!("  - {m}");
    }
    eprintln!();
    eprintln!("Fix by any of:");
    eprintln!("  export SHOT_CONFIG_{section_upper}_API_KEY=<key>");
    eprintln!("  shot --config.{provider}.api_key=<key> \"...\"");
    eprintln!("  shot --config.{provider}.api_key=<key> config show > ~/.config/shot/agent.toml");
    if let Some(url) = key_url(provider) {
        eprintln!();
        eprintln!("Get a key: {url}");
    }
    std::process::exit(1);
}

fn die_ambiguous_provider(configured: &[String]) -> ! {
    let example = configured.first().map(String::as_str).unwrap_or("gemini");
    eprintln!("Multiple providers have keys set: {}.", configured.join(", "));
    eprintln!();
    eprintln!("Pick one by any of:");
    eprintln!("  export SHOT_CONFIG_AGENT_PROVIDER={example}");
    eprintln!("  shot --config.agent.provider={example} \"...\"");
    eprintln!("  add `provider = \"{example}\"` to [agent] in ~/.config/shot/agent.toml");
    std::process::exit(1);
}

fn die_no_provider_configured() -> ! {
    eprintln!("No provider configured.");
    eprintln!();
    eprintln!("Get a key:");
    for (name, url) in KNOWN_PROVIDERS {
        eprintln!("  {name:<10} {url}");
    }
    eprintln!();
    eprintln!("Provide it by any of:");
    eprintln!("  env var   export SHOT_CONFIG_<PROVIDER>_API_KEY=<key>");
    eprintln!("  CLI flag  shot --config.<provider>.api_key=<key> \"...\"");
    eprintln!("  file      shot --config.<provider>.api_key=<key> config show > ~/.config/shot/agent.toml");
    eprintln!();
    eprintln!("Shot auto-picks the provider from whichever key is set.");
    eprintln!("To force one: SHOT_CONFIG_AGENT_PROVIDER=<name> or --config.agent.provider=<name>");
    std::process::exit(1);
}

const KNOWN_PROVIDERS: &[(&str, &str)] = &[
    ("gemini",    "https://aistudio.google.com/apikey"),
    ("openai",    "https://platform.openai.com/api-keys"),
    ("anthropic", "https://console.anthropic.com/settings/keys"),
];

fn key_url(provider: &str) -> Option<&'static str> {
    KNOWN_PROVIDERS.iter()
        .find(|(n, _)| *n == provider)
        .map(|(_, u)| *u)
}
