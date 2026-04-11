use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

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
    #[serde(default)]
    roles: HashMap<String, RoleConfigFile>,
}

fn default_max_turns() -> usize { 50 }

#[derive(Deserialize)]
struct ProviderConfig {
    llm_url: String,
    api_key: String,
    model: String,
    #[serde(default)]
    reasoning: Option<String>,
}

#[derive(Clone, Deserialize)]
struct RoleConfigFile {
    #[serde(default)]
    prompt: String,
    #[serde(default)]
    tools: Vec<String>,
    #[serde(default = "default_color")]
    color: String,
}

fn default_color() -> String { "white".into() }

// ── Public types ───────────────────────────────────────────────────────

#[derive(Clone)]
pub struct RoleConfig {
    pub prompt: String,
    pub tools: Vec<String>,
    pub color: String,
}

pub struct Config {
    pub llm_url: String,
    pub api_key: String,
    pub model: String,
    pub reasoning: Option<String>,
    pub soul_prompt: String,
    pub max_turns: usize,
    pub tools_dir: String,
    pub roles: HashMap<String, RoleConfig>,
}

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

fn resolve(base: &PathBuf, path: &str) -> String {
    if path.is_empty() { return String::new(); }
    let p = PathBuf::from(path);
    if p.is_absolute() { path.to_string() } else { base.join(path).to_string_lossy().to_string() }
}

// ── Load ───────────────────────────────────────────────────────────────

impl Config {
    pub fn load() -> Self {
        let path = config_path();
        if !path.exists() {
            eprintln!("Not configured. Run: shot configure <provider> --api-key <key>");
            eprintln!("Supported providers: gemini");
            std::process::exit(1);
        }

        let data_dir = data_dir();
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("Failed to read {}: {e}", path.display()));
        let file: ConfigFile = toml::from_str(&raw)
            .unwrap_or_else(|e| panic!("Failed to parse {}: {e}", path.display()));

        let provider_name = &file.agent.provider;
        let provider_val = file.providers.get(provider_name)
            .unwrap_or_else(|| panic!("Provider '{}' not found in {}", provider_name, path.display()));
        let provider: ProviderConfig = provider_val.clone().try_into()
            .unwrap_or_else(|e| panic!("Failed to parse provider '{}': {e}", provider_name));

        let soul_file = resolve(&data_dir, &file.agent.soul_file);
        let tools_dir = if file.agent.tools_dir.is_empty() {
            data_dir.join("tools").to_string_lossy().to_string()
        } else {
            resolve(&data_dir, &file.agent.tools_dir)
        };

        let roles = file.agent.roles.into_iter().map(|(name, rc)| {
            (name, RoleConfig {
                prompt: rc.prompt,
                tools: rc.tools,
                color: rc.color,
            })
        }).collect();

        Self {
            llm_url: provider.llm_url,
            api_key: provider.api_key,
            model: provider.model,
            reasoning: provider.reasoning,
            soul_prompt: if soul_file.is_empty() { String::new() } else {
                std::fs::read_to_string(&soul_file).unwrap_or_default()
            },
            max_turns: file.agent.max_turns,
            tools_dir,
            roles,
        }
    }
}
