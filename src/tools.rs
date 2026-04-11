use crate::react::{FunctionDef, ToolDef, ToolExecutor};
use serde::Deserialize;
use std::collections::HashMap;

// ── Tool spec (loaded from TOML) ───────────────────────────────────────

#[derive(Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub command: String,
    #[serde(default)]
    pub healthcheck: Option<String>,
    #[serde(default)]
    pub stdin: Option<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub params: HashMap<String, ParamSpec>,
}

#[derive(Deserialize)]
pub struct ParamSpec {
    #[serde(rename = "type", default = "default_type")]
    pub param_type: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub required: bool,
}

fn default_type() -> String { "string".into() }

impl ToolSpec {
    pub fn to_tool_def(&self) -> ToolDef {
        let mut properties = serde_json::Map::new();
        let mut required = Vec::new();

        for (name, param) in &self.params {
            properties.insert(name.clone(), serde_json::json!({
                "type": param.param_type,
                "description": param.description,
            }));
            if param.required {
                required.push(serde_json::Value::String(name.clone()));
            }
        }

        ToolDef {
            kind: "function".into(),
            function: FunctionDef {
                name: self.name.clone(),
                description: self.description.clone(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": properties,
                    "required": required,
                }),
            },
        }
    }

    fn apply_env(&self, cmd: &mut tokio::process::Command) {
        for (key, value) in &self.env {
            cmd.env(key, value);
        }
    }

    fn apply_env_sync(&self, cmd: &mut std::process::Command) {
        for (key, value) in &self.env {
            cmd.env(key, value);
        }
    }

    fn apply_args(&self, cmd: &mut tokio::process::Command, args: &serde_json::Value) {
        if let Some(obj) = args.as_object() {
            for (key, value) in obj {
                let val_str = match value {
                    serde_json::Value::String(s) => s.clone(),
                    serde_json::Value::Bool(b) => b.to_string(),
                    serde_json::Value::Number(n) => n.to_string(),
                    serde_json::Value::Null => String::new(),
                    other => other.to_string(),
                };
                cmd.env(key, &val_str);
            }
        }
    }

    pub async fn execute(&self, args: &serde_json::Value) -> String {
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c").arg(&self.command);

        self.apply_env(&mut cmd);
        self.apply_args(&mut cmd, args);

        if let Some(ref stdin_tpl) = self.stdin {
            let mut stdin_val = stdin_tpl.clone();
            if let Some(obj) = args.as_object() {
                for (key, value) in obj {
                    let val_str = match value {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    stdin_val = stdin_val.replace(&format!("{{{key}}}"), &val_str);
                }
            }
            cmd.stdin(std::process::Stdio::piped());
            let mut child = match cmd.spawn() {
                Ok(c) => c,
                Err(e) => return format!("Failed to execute: {e}"),
            };
            if let Some(ref mut stdin) = child.stdin {
                use tokio::io::AsyncWriteExt;
                let _ = stdin.write_all(stdin_val.as_bytes()).await;
            }
            child.stdin.take();
            match child.wait_with_output().await {
                Ok(output) => format_output(&output),
                Err(e) => format!("Failed to execute: {e}"),
            }
        } else {
            match cmd.output().await {
                Ok(output) => format_output(&output),
                Err(e) => format!("Failed to execute: {e}"),
            }
        }
    }
}

fn format_output(output: &std::process::Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    if output.status.success() {
        if stdout.is_empty() { "(no output)".into() } else { stdout }
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let msg = if stderr.is_empty() { &stdout } else { &stderr };
        format!("Error (exit {}): {}", output.status.code().unwrap_or(-1), msg)
    }
}

// ── External tools executor ────────────────────────────────────────────

pub struct ExternalTools {
    tools: Vec<ToolSpec>,
}

impl ExternalTools {
    pub fn load(tools_dir: &str, names: &[String]) -> Self {
        let dir = std::path::Path::new(tools_dir);
        let mut tools = Vec::new();

        for name in names {
            let path = dir.join(format!("{name}.toml"));
            if let Ok(content) = std::fs::read_to_string(&path) {
                match toml::from_str::<ToolSpec>(&content) {
                    Ok(spec) => {
                        if let Some(ref check) = spec.healthcheck {
                            let mut cmd = std::process::Command::new("sh");
                            cmd.arg("-c").arg(check);
                            cmd.stdout(std::process::Stdio::null());
                            cmd.stderr(std::process::Stdio::null());
                            spec.apply_env_sync(&mut cmd);
                            let ok = cmd.status().map(|s| s.success()).unwrap_or(false);
                            if !ok {
                                continue;
                            }
                        }
                        tools.push(spec);
                    }
                    Err(e) => eprintln!("Warning: failed to parse {}: {e}", path.display()),
                }
            }
        }

        Self { tools }
    }
}

pub fn healthcheck_all(tools_dir: &str) {
    let dir = std::path::Path::new(tools_dir);
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().map(|x| x == "toml").unwrap_or(false))
        .collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let path = entry.path();
        let name = path.file_stem().unwrap_or_default().to_string_lossy();
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => { println!("  ? {name}  (unreadable)"); continue; }
        };
        let spec: ToolSpec = match toml::from_str(&content) {
            Ok(s) => s,
            Err(e) => { println!("  ? {name}  (parse error: {e})"); continue; }
        };
        match &spec.healthcheck {
            None => println!("  \x1b[32m✓\x1b[0m {name}  (no healthcheck)"),
            Some(check) => {
                let mut cmd = std::process::Command::new("sh");
                cmd.arg("-c").arg(check);
                cmd.stdout(std::process::Stdio::null());
                cmd.stderr(std::process::Stdio::null());
                spec.apply_env_sync(&mut cmd);
                let ok = cmd.status().map(|s| s.success()).unwrap_or(false);
                if ok {
                    println!("  \x1b[32m✓\x1b[0m {name}");
                } else {
                    println!("  \x1b[31m✗\x1b[0m {name}  ({check})");
                }
            }
        }
    }
}

impl ToolExecutor for ExternalTools {
    fn definitions(&self) -> Vec<ToolDef> {
        self.tools.iter().map(|t| t.to_tool_def()).collect()
    }

    async fn execute(&self, name: &str, args: &serde_json::Value) -> String {
        for tool in &self.tools {
            if tool.name == name {
                return tool.execute(args).await;
            }
        }
        format!("Unknown tool: {name}")
    }
}
