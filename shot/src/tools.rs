use crate::react::{FunctionDef, ToolDef, ToolExecutor};
use serde::Deserialize;
use std::collections::HashMap;

// ── Tool spec (loaded from TOML) ───────────────────────────────────────

#[derive(Deserialize, Clone)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub command: String,
    #[serde(default)]
    pub healthcheck: Option<String>,
    #[serde(default)]
    pub vars: HashMap<String, VarSpec>,

    /// Runtime overrides set via CLI flags. Not from TOML.
    #[serde(skip)]
    pub fixed_vars: HashMap<String, String>,
}

#[derive(Deserialize, Clone)]
pub struct VarSpec {
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub default: Option<String>,
    /// Fixed value set in the TOML. Like a CLI override but baked into the tool.
    /// CLI overrides take precedence over this.
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub hide: bool,
    #[serde(rename = "type", default = "default_type")]
    pub var_type: String,
}

fn default_type() -> String { "string".into() }

impl ToolSpec {
    pub fn to_tool_def(&self) -> ToolDef {
        let mut properties = serde_json::Map::new();
        let mut required = Vec::new();

        for (name, var) in &self.vars {
            // Hidden vars never appear in the LLM schema
            if var.hide { continue; }

            let mut description = var.description.clone();
            if let Some(fixed_val) = self.fixed_vars.get(name) {
                description = if description.is_empty() {
                    format!("(restricted to: {})", fixed_val)
                } else {
                    format!("{} (restricted to: {})", description, fixed_val)
                };
            }

            properties.insert(name.clone(), serde_json::json!({
                "type": var.var_type,
                "description": description,
            }));

            // Fixed vars don't need to be required — they're already set
            if var.required && !self.fixed_vars.contains_key(name) {
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

    fn apply_env_sync(&self, cmd: &mut std::process::Command) {
        for (key, value) in &self.fixed_vars {
            cmd.env(key, value);
        }
        for (name, var) in &self.vars {
            if let Some(default) = &var.default {
                if !self.fixed_vars.contains_key(name) {
                    cmd.env(name, default);
                }
            }
        }
    }

    pub async fn execute(&self, args: &serde_json::Value) -> String {
        // Validate that the LLM didn't try to override fixed vars with different values
        if let Some(obj) = args.as_object() {
            for (key, value) in obj {
                if let Some(fixed_val) = self.fixed_vars.get(key) {
                    let provided = match value {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    if provided != *fixed_val {
                        return format!(
                            "Error: parameter '{}' is restricted to '{}' (you provided '{}')",
                            key, fixed_val, provided
                        );
                    }
                }
            }
        }

        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c").arg(&self.command);

        // Set fixed vars (from CLI overrides) — these always win
        for (key, value) in &self.fixed_vars {
            cmd.env(key, value);
        }

        // Set vars from LLM args (skip ones that are fixed)
        if let Some(obj) = args.as_object() {
            for (key, value) in obj {
                if self.fixed_vars.contains_key(key) { continue; }
                let val_str = match value {
                    serde_json::Value::String(s) => s.clone(),
                    serde_json::Value::Bool(b) => b.to_string(),
                    serde_json::Value::Number(n) => n.to_string(),
                    serde_json::Value::Null => String::new(),
                    other => other.to_string(),
                };
                cmd.env(key, val_str);
            }
        }

        // Apply defaults for vars not provided by LLM and not fixed
        for (name, var) in &self.vars {
            if self.fixed_vars.contains_key(name) { continue; }
            if args.get(name).is_some() { continue; }
            if let Some(default) = &var.default {
                cmd.env(name, default);
            }
        }

        match cmd.output().await {
            Ok(output) => format_output(&output),
            Err(e) => format!("Failed to execute: {e}"),
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
    /// Load tools from disk.
    /// - `enabled`: if Some, only load tools with these names. If None, load all *.toml files.
    /// - `overrides`: per-tool var overrides from CLI flags.
    pub fn load(
        tools_dir: &str,
        enabled: Option<&[String]>,
        overrides: &HashMap<String, HashMap<String, String>>,
    ) -> Self {
        let dir = std::path::Path::new(tools_dir);
        let mut tools = Vec::new();

        // Determine which tool names to load
        let names: Vec<String> = match enabled {
            Some(list) => list.to_vec(),
            None => std::fs::read_dir(dir)
                .into_iter()
                .flatten()
                .flatten()
                .filter_map(|e| {
                    let p = e.path();
                    if p.extension()? == "toml" {
                        Some(p.file_stem()?.to_string_lossy().to_string())
                    } else { None }
                })
                .collect(),
        };

        for name in &names {
            let path = dir.join(format!("{name}.toml"));
            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let mut spec: ToolSpec = match toml::from_str(&content) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Warning: failed to parse {}: {e}", path.display());
                    continue;
                }
            };

            // Apply TOML `value` fields first (lowest priority of fixed)
            for (var_name, var) in &spec.vars {
                if let Some(val) = &var.value {
                    spec.fixed_vars.insert(var_name.clone(), val.clone());
                }
            }

            // Apply CLI overrides (highest priority — overwrites TOML value)
            if let Some(tool_overrides) = overrides.get(name) {
                for (k, v) in tool_overrides {
                    spec.fixed_vars.insert(k.clone(), v.clone());
                }
            }

            // Check that all hidden+required vars have a value (either fixed or default)
            let mut missing_hidden = Vec::new();
            for (var_name, var) in &spec.vars {
                if var.hide && var.required
                    && !spec.fixed_vars.contains_key(var_name)
                    && var.default.is_none()
                {
                    missing_hidden.push(var_name.clone());
                }
            }
            if !missing_hidden.is_empty() {
                continue;
            }

            // Healthcheck
            if let Some(ref check) = spec.healthcheck {
                let mut cmd = std::process::Command::new("sh");
                cmd.arg("-c").arg(check);
                cmd.stdout(std::process::Stdio::null());
                cmd.stderr(std::process::Stdio::null());
                spec.apply_env_sync(&mut cmd);
                let ok = cmd.status().map(|s| s.success()).unwrap_or(false);
                if !ok { continue; }
            }

            tools.push(spec);
        }

        Self { tools }
    }
}

pub fn healthcheck_all(tools_dir: &str, overrides: &HashMap<String, HashMap<String, String>>) {
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
        let name = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => { println!("  ? {name}  (unreadable)"); continue; }
        };
        let mut spec: ToolSpec = match toml::from_str(&content) {
            Ok(s) => s,
            Err(e) => { println!("  ? {name}  (parse error: {e})"); continue; }
        };

        // Apply TOML `value` fields
        for (var_name, var) in &spec.vars {
            if let Some(val) = &var.value {
                spec.fixed_vars.insert(var_name.clone(), val.clone());
            }
        }
        // Apply CLI overrides
        if let Some(tool_overrides) = overrides.get(&name) {
            for (k, v) in tool_overrides {
                spec.fixed_vars.insert(k.clone(), v.clone());
            }
        }

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
