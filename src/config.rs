use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Deserialize)]
struct ConfigFile {
    agent: AgentConfig,
    #[serde(flatten)]
    providers: HashMap<String, ProviderConfig>,
}

#[derive(Deserialize)]
struct AgentConfig {
    provider: String,
    #[serde(default)]
    soul_file: String,
    #[serde(default)]
    skills_dir: String,
    #[serde(default = "default_max_turns")]
    max_turns: usize,
    #[serde(default = "default_memory_db")]
    memory_db: String,
    #[serde(default = "default_session_path")]
    session_path: String,
    #[serde(default = "default_max_session_chars")]
    max_session_chars: usize,
}

fn default_max_turns() -> usize { 50 }
fn default_memory_db() -> String { "memory.db".into() }
fn default_session_path() -> String { "session.db".into() }
fn default_max_session_chars() -> usize { 200_000 }

#[derive(Deserialize)]
struct ProviderConfig {
    llm_url: String,
    embed_url: String,
    api_key: String,
    model: String,
    reasoning: ReasoningConfig,
}

#[derive(Deserialize)]
struct ReasoningConfig {
    planner: String,
    executor: String,
    supervisor: String,
}

pub struct Config {
    pub llm_url: String,
    pub embed_url: String,
    pub api_key: String,
    pub model: String,
    pub planner_reasoning: String,
    pub executor_reasoning: String,
    pub supervisor_reasoning: String,
    pub soul_prompt: String,
    pub skills_prompt: String,
    pub max_turns: usize,
    pub memory_db: String,
    pub session_path: String,
    pub max_session_chars: usize,
}

fn config_dir() -> PathBuf {
    home_dir().join(".config").join("shot")
}

fn data_dir() -> PathBuf {
    home_dir().join(".local").join("share").join("shot")
}

fn home_dir() -> PathBuf {
    std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("."))
}

fn config_path() -> PathBuf {
    config_dir().join("agent.toml")
}

fn resolve(base: &PathBuf, path: &str) -> String {
    if path.is_empty() {
        return String::new();
    }
    let p = PathBuf::from(path);
    if p.is_absolute() {
        path.to_string()
    } else {
        base.join(path).to_string_lossy().to_string()
    }
}

fn load_skills(dir: &str) -> String {
    if dir.is_empty() { return String::new(); }
    let skills_dir = std::path::Path::new(dir);
    let mut prompt = String::new();
    if let Ok(entries) = std::fs::read_dir(skills_dir) {
        for entry in entries.flatten() {
            if let Ok(s) = std::fs::read_to_string(entry.path().join("SKILL.md")) {
                prompt.push_str(&s);
                prompt.push('\n');
            }
        }
    }
    prompt
}

const DEFAULT_SOUL: &str = r#"# Soul

You are Shot — a personal AI assistant.

## Personality

Be genuinely helpful, not performatively helpful. Skip filler — just help.
Have opinions. Be resourceful before asking.
Act, don't lecture. Research → plan → execute → report.

## Conciseness

- Max 2-3 short paragraphs. No walls of text.
- Use bullet points, not essays.
- If you can say it in one sentence, don't use three.
- State the result, not the process.
"#;

struct ProviderTemplate {
    llm_url: &'static str,
    embed_url: &'static str,
    model: &'static str,
}

fn provider_template(name: &str) -> Option<ProviderTemplate> {
    match name {
        "gemini" => Some(ProviderTemplate {
            llm_url: "https://generativelanguage.googleapis.com/v1beta/openai",
            embed_url: "https://generativelanguage.googleapis.com/v1beta/models/gemini-embedding-001:embedContent",
            model: "gemini-2.5-flash",
        }),
        _ => None,
    }
}

pub fn configure(provider: &str, api_key: &str) {
    let Some(template) = provider_template(provider) else {
        eprintln!("Unknown provider: {provider}");
        eprintln!("Supported: gemini");
        std::process::exit(1);
    };

    let config_dir = config_dir();
    let data_dir = data_dir();
    let _ = std::fs::create_dir_all(&config_dir);
    let _ = std::fs::create_dir_all(&data_dir);
    let _ = std::fs::create_dir_all(data_dir.join("skills"));

    let d = data_dir.display();
    let config = format!(r#"[{provider}]
llm_url = "{}"
embed_url = "{}"
api_key = "{api_key}"
model = "{}"

[{provider}.reasoning]
planner = "high"
executor = "low"
supervisor = "low"

[agent]
provider = "{provider}"
soul_file = "{d}/SOUL.md"
skills_dir = "{d}/skills"
max_turns = 50
memory_db = "{d}/memory.db"
session_path = "{d}/session.db"
max_session_chars = 200000
"#, template.llm_url, template.embed_url, template.model);

    let path = config_path();
    std::fs::write(&path, config).expect("Failed to write config");
    println!("Config written to {}", path.display());

    let soul_path = data_dir.join("SOUL.md");
    if !soul_path.exists() {
        std::fs::write(&soul_path, DEFAULT_SOUL).expect("Failed to write SOUL.md");
        println!("Soul written to {}", soul_path.display());
    }

    println!("Ready. Run: shot \"hello\"");
}

impl Config {
    pub fn load() -> Self {
        let path = config_path();
        if !path.exists() {
            eprintln!("Not configured. Run: shot configure <provider> --api-key <key>");
            eprintln!("Supported providers: gemini");
            std::process::exit(1);
        }

        let data_dir = data_dir();

        let file: ConfigFile = std::fs::read_to_string(&path)
            .map_err(|e| format!("Failed to read {}: {e}", path.display()))
            .and_then(|s| toml::from_str(&s).map_err(|e| format!("Failed to parse {}: {e}", path.display())))
            .expect(&format!("{} is required", path.display()));

        let provider_name = &file.agent.provider;
        let provider = file.providers.get(provider_name)
            .unwrap_or_else(|| panic!("Provider '{}' not found in {}", provider_name, path.display()));

        let soul_file = resolve(&data_dir, &file.agent.soul_file);
        let skills_dir = resolve(&data_dir, &file.agent.skills_dir);
        let memory_db = resolve(&data_dir, &file.agent.memory_db);
        let session_path = resolve(&data_dir, &file.agent.session_path);

        Self {
            llm_url: provider.llm_url.clone(),
            embed_url: provider.embed_url.clone(),
            api_key: provider.api_key.clone(),
            model: provider.model.clone(),
            planner_reasoning: provider.reasoning.planner.clone(),
            executor_reasoning: provider.reasoning.executor.clone(),
            supervisor_reasoning: provider.reasoning.supervisor.clone(),
            soul_prompt: if soul_file.is_empty() { String::new() } else {
                std::fs::read_to_string(&soul_file).unwrap_or_default()
            },
            skills_prompt: load_skills(&skills_dir),
            max_turns: file.agent.max_turns,
            memory_db,
            session_path,
            max_session_chars: file.agent.max_session_chars,
        }
    }
}
