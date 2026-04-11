use std::path::PathBuf;

fn home_dir() -> PathBuf {
    std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("."))
}

const DEFAULT_CONFIG: &str = include_str!("../defaults/agent.toml");
const DEFAULT_SOUL: &str = include_str!("../defaults/SOUL.md");

const DEFAULT_TOOLS: &[(&str, &str)] = &[
    ("file_read.toml", include_str!("../defaults/tools/file_read.toml")),
    ("file_write.toml", include_str!("../defaults/tools/file_write.toml")),
    ("list_files.toml", include_str!("../defaults/tools/list_files.toml")),
    ("search_text.toml", include_str!("../defaults/tools/search_text.toml")),
    ("shell.toml", include_str!("../defaults/tools/shell.toml")),
    ("memory_store.toml", include_str!("../defaults/tools/memory_store.toml")),
    ("memory_recall.toml", include_str!("../defaults/tools/memory_recall.toml")),
    ("web_search.toml", include_str!("../defaults/tools/web_search.toml")),
    ("web_read.toml", include_str!("../defaults/tools/web_read.toml")),
];

struct ProviderTemplate {
    llm_url: &'static str,
    model: &'static str,
}

fn provider_template(name: &str) -> Option<ProviderTemplate> {
    match name {
        "gemini" => Some(ProviderTemplate {
            llm_url: "https://generativelanguage.googleapis.com/v1beta/openai",
            model: "gemini-3-flash-preview",
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

    let config_dir = home_dir().join(".config/shot");
    let data_dir = home_dir().join(".local/share/shot");
    let tools_dir = data_dir.join("tools");
    let _ = std::fs::create_dir_all(&config_dir);
    let _ = std::fs::create_dir_all(&tools_dir);

    // Write config
    let config = DEFAULT_CONFIG
        .replace("{{provider}}", provider)
        .replace("{{llm_url}}", template.llm_url)
        .replace("{{api_key}}", api_key)
        .replace("{{model}}", template.model)
        .replace("{{data_dir}}", &data_dir.display().to_string());

    let path = config_dir.join("agent.toml");
    std::fs::write(&path, config).expect("Failed to write config");
    println!("Config written to {}", path.display());

    // Write soul
    let soul_path = data_dir.join("SOUL.md");
    if !soul_path.exists() {
        std::fs::write(&soul_path, DEFAULT_SOUL).expect("Failed to write SOUL.md");
        println!("Soul written to {}", soul_path.display());
    }

    // Write default tools
    for (name, content) in DEFAULT_TOOLS {
        let p = tools_dir.join(name);
        if !p.exists() {
            let _ = std::fs::write(&p, content);
        }
    }
    println!("Tools written to {}", tools_dir.display());

    println!("Ready. Run: shot \"hello\"");
}
