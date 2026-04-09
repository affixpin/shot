use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize)]
struct ConfigFile {
    agent: AgentConfig,
    #[serde(flatten)]
    providers: HashMap<String, ProviderConfig>,
}

#[derive(Deserialize)]
struct AgentConfig {
    provider: String,
    soul_file: String,
    skills_dir: String,
    max_turns: usize,
    memory_db: String,
    session_path: String,
    max_session_chars: usize,
}

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

fn load_skills(dir: &str) -> String {
    let skills_dir = std::path::Path::new(dir);
    let mut prompt = String::new();
    if let Ok(entries) = std::fs::read_dir(&skills_dir) {
        for entry in entries.flatten() {
            if let Ok(s) = std::fs::read_to_string(entry.path().join("SKILL.md")) {
                prompt.push_str(&s);
                prompt.push('\n');
            }
        }
    }
    prompt
}

impl Config {
    pub fn load() -> Self {
        let file: ConfigFile = std::fs::read_to_string("agent.toml")
            .map_err(|e| format!("Failed to read agent.toml: {e}"))
            .and_then(|s| toml::from_str(&s).map_err(|e| format!("Failed to parse agent.toml: {e}")))
            .expect("agent.toml is required");

        let provider_name = &file.agent.provider;
        let provider = file.providers.get(provider_name)
            .unwrap_or_else(|| panic!("Provider '{}' not found in agent.toml", provider_name));

        Self {
            llm_url: provider.llm_url.clone(),
            embed_url: provider.embed_url.clone(),
            api_key: provider.api_key.clone(),
            model: provider.model.clone(),
            planner_reasoning: provider.reasoning.planner.clone(),
            executor_reasoning: provider.reasoning.executor.clone(),
            supervisor_reasoning: provider.reasoning.supervisor.clone(),
            soul_prompt: std::fs::read_to_string(&file.agent.soul_file).unwrap_or_default(),
            skills_prompt: load_skills(&file.agent.skills_dir),
            max_turns: file.agent.max_turns,
            memory_db: file.agent.memory_db,
            session_path: file.agent.session_path,
            max_session_chars: file.agent.max_session_chars,
        }
    }
}
