use crate::memory::Memory;
use crate::react::{FunctionDef, ToolDef, ToolExecutor};
use std::process::Stdio;
use std::sync::Arc;
use tokio::process::Command;

// ── Secret redaction ────────────────────────────────────────────────────

fn redact_secrets(input: &str) -> String {
    use std::sync::OnceLock;
    use regex::Regex;

    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    let patterns = PATTERNS.get_or_init(|| {
        [
            r"AIza[0-9A-Za-z_-]{35}",                         // Google API key
            r"sk-[0-9a-zA-Z]{20,}",                           // OpenAI key
            r"ghp_[0-9a-zA-Z]{36}",                           // GitHub personal token
            r"gho_[0-9a-zA-Z]{36}",                           // GitHub OAuth token
            r"github_pat_[0-9a-zA-Z_]{82}",                   // GitHub fine-grained token
            r"xoxb-[0-9]{10,}-[0-9a-zA-Z]+",                  // Slack bot token
            r"xoxp-[0-9]{10,}-[0-9a-zA-Z]+",                  // Slack user token
            r"(?i)bearer\s+[0-9a-zA-Z_.~+/-]+=*",             // Bearer tokens
            r"-----BEGIN (?:RSA |EC )?PRIVATE KEY-----",       // Private keys
            r#"(?i)(?:api[_-]?key|api[_-]?secret|password|secret[_-]?key|access[_-]?token)\s*[=:]\s*['"]?[0-9a-zA-Z_./-]{16,}['"]?"#, // Generic key=value
        ]
        .iter()
        .filter_map(|p| Regex::new(p).ok())
        .collect()
    });

    let mut result = input.to_string();
    for pattern in patterns {
        result = pattern.replace_all(&result, "***REDACTED***").to_string();
    }
    result
}

// ── Shared tool definitions ─────────────────────────────────────────────

fn shell_tool() -> ToolDef {
    ToolDef {
        kind: "function".into(),
        function: FunctionDef {
            name: "shell".into(),
            description: "Execute a shell command. Killed after timeout.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string" },
                    "timeout": { "type": "integer", "description": "Timeout in seconds (default 60, max 300)" }
                },
                "required": ["command"]
            }),
        },
    }
}

fn file_read_tool() -> ToolDef {
    ToolDef {
        kind: "function".into(),
        function: FunctionDef {
            name: "file_read".into(),
            description: "Read a file".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"]
            }),
        },
    }
}

fn file_write_tool() -> ToolDef {
    ToolDef {
        kind: "function".into(),
        function: FunctionDef {
            name: "file_write".into(),
            description: "Write content to a file".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "content": { "type": "string" }
                },
                "required": ["path", "content"]
            }),
        },
    }
}

fn memory_store_tool() -> ToolDef {
    ToolDef {
        kind: "function".into(),
        function: FunctionDef {
            name: "memory_store".into(),
            description: "Store a fact or preference to long-term memory. Use for things worth remembering across conversations.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "key": { "type": "string", "description": "Short identifier like 'user_lang' or 'project_name'" },
                    "content": { "type": "string", "description": "The fact to remember" }
                },
                "required": ["key", "content"]
            }),
        },
    }
}

fn memory_recall_tool() -> ToolDef {
    ToolDef {
        kind: "function".into(),
        function: FunctionDef {
            name: "memory_recall".into(),
            description: "Search long-term memory for relevant facts. Returns semantically similar memories.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "What to search for" }
                },
                "required": ["query"]
            }),
        },
    }
}

fn send_file_tool() -> ToolDef {
    ToolDef {
        kind: "function".into(),
        function: FunctionDef {
            name: "send_file".into(),
            description: "Send a file to the user. The user cannot see files in your workspace — use this to deliver any file they need to see (images, documents, code, etc).".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the file to send" },
                    "caption": { "type": "string", "description": "Short description of the file" }
                },
                "required": ["path"]
            }),
        },
    }
}

fn list_files_tool() -> ToolDef {
    ToolDef {
        kind: "function".into(),
        function: FunctionDef {
            name: "list_files".into(),
            description: "List files in the workspace. Respects .gitignore automatically. Use this instead of ls/find/fd.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Directory to list (default: current directory)" },
                    "recursive": { "type": "boolean", "description": "List recursively (default: false)" },
                    "pattern": { "type": "string", "description": "Glob pattern filter, e.g. '*.md', '*.{rs,toml}', 'src/**/*.rs'" },
                    "ext": { "type": "string", "description": "Filter by file extension, e.g. 'rs', 'md', 'ts'" },
                    "max_depth": { "type": "integer", "description": "Max directory depth (only with recursive)" },
                    "hidden": { "type": "boolean", "description": "Include hidden files/dirs (default: false)" },
                    "dirs_only": { "type": "boolean", "description": "List only directories, not files (default: false)" },
                    "max_results": { "type": "integer", "description": "Maximum number of results to return (default: 1000)" }
                }
            }),
        },
    }
}

fn search_text_tool() -> ToolDef {
    ToolDef {
        kind: "function".into(),
        function: FunctionDef {
            name: "search_text".into(),
            description: "Search file contents by regex pattern. Respects .gitignore. Returns matching lines with file paths and line numbers. Use this instead of grep/rg.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "Regex pattern to search for" },
                    "path": { "type": "string", "description": "Directory or file to search in (default: current directory)" },
                    "ext": { "type": "string", "description": "Filter by file extension, e.g. 'rs', 'md'" },
                    "glob": { "type": "string", "description": "Glob pattern to filter files, e.g. '*.toml', 'src/**/*.rs'" },
                    "case_insensitive": { "type": "boolean", "description": "Case-insensitive search (default: false)" },
                    "hidden": { "type": "boolean", "description": "Search hidden files/dirs (default: false)" },
                    "max_results": { "type": "integer", "description": "Maximum number of matching lines to return (default: 500)" },
                    "context_lines": { "type": "integer", "description": "Number of context lines before and after each match (default: 0)" }
                },
                "required": ["pattern"]
            }),
        },
    }
}

fn create_plan_tool() -> ToolDef {
    ToolDef {
        kind: "function".into(),
        function: FunctionDef {
            name: "create_plan".into(),
            description: "Submit the execution plan. Call this when you are ready with your plan.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "steps": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Ordered list of steps for the executor to perform"
                    }
                },
                "required": ["steps"]
            }),
        },
    }
}

// ── Tool execution (shared logic) ───────────────────────────────────────

async fn execute_tool(name: &str, args: &serde_json::Value, memory: &Option<Arc<Memory>>) -> String {
    redact_secrets(&execute_tool_raw(name, args, memory).await)
}

async fn execute_tool_raw(name: &str, args: &serde_json::Value, memory: &Option<Arc<Memory>>) -> String {
    match name {
        "shell" => {
            let cmd = args["command"].as_str().unwrap_or("");
            let timeout_secs = args["timeout"].as_u64().unwrap_or(60).min(300);

            let fut = Command::new("bash")
                .args(["-c", cmd])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output();

            let timeout = std::time::Duration::from_secs(timeout_secs);
            match tokio::time::timeout(timeout, fut).await {
                Ok(Ok(out)) => {
                    let mut r = String::from_utf8_lossy(&out.stdout).to_string();
                    let err = String::from_utf8_lossy(&out.stderr);
                    if !err.is_empty() {
                        if !r.is_empty() { r.push('\n'); }
                        r.push_str(&err);
                    }
                    if r.len() > 50_000 { r.truncate(50_000); r.push_str("\n[truncated]"); }
                    if r.is_empty() { "(no output)".into() } else { r }
                }
                Ok(Err(e)) => format!("Error: {e}"),
                Err(_) => format!("Error: command timed out after {timeout_secs}s"),
            }
        }
        "file_read" => {
            let path = args["path"].as_str().unwrap_or("");
            std::fs::read_to_string(path).unwrap_or_else(|e| format!("Error: {e}"))
        }
        "file_write" => {
            let path = args["path"].as_str().unwrap_or("");
            let content = args["content"].as_str().unwrap_or("");
            if let Some(p) = std::path::Path::new(path).parent() {
                let _ = std::fs::create_dir_all(p);
            }
            match std::fs::write(path, content) {
                Ok(_) => format!("Written to {path}"),
                Err(e) => format!("Error: {e}"),
            }
        }
        "memory_store" => {
            let Some(ref mem) = memory else { return "Memory not available".into() };
            let key = args["key"].as_str().unwrap_or("");
            let content = args["content"].as_str().unwrap_or("");
            match mem.store(key, content).await {
                Ok(_) => format!("Stored memory: {key}"),
                Err(e) => format!("Error storing memory: {e}"),
            }
        }
        "memory_recall" => {
            let Some(ref mem) = memory else { return "Memory not available".into() };
            let query = args["query"].as_str().unwrap_or("");
            match mem.recall(query, 5).await {
                Ok(entries) if entries.is_empty() => "No relevant memories found.".into(),
                Ok(entries) => entries.iter()
                    .map(|e| format!("- {} (score: {:.2}): {}", e.key, e.score, e.content))
                    .collect::<Vec<_>>()
                    .join("\n"),
                Err(e) => format!("Error recalling memory: {e}"),
            }
        }
        "send_file" => {
            let path = args["path"].as_str().unwrap_or("");
            if path.is_empty() {
                return "Error: path is required".into();
            }
            if !std::path::Path::new(path).exists() {
                return format!("Error: file not found: {path}");
            }
            let caption = args["caption"].as_str().unwrap_or("");
            crate::emit::emit_system("file", serde_json::json!({
                "path": path,
                "caption": caption,
            }));
            format!("File sent: {path}")
        }
        "list_files" => {
            use ignore::WalkBuilder;
            use ignore::overrides::OverrideBuilder;

            let path = args["path"].as_str().unwrap_or(".");
            let recursive = args["recursive"].as_bool().unwrap_or(false);
            let pattern = args["pattern"].as_str().unwrap_or("");
            let ext = args["ext"].as_str().unwrap_or("");
            let max_depth = args["max_depth"].as_u64().map(|d| d as usize);
            let hidden = args["hidden"].as_bool().unwrap_or(false);
            let dirs_only = args["dirs_only"].as_bool().unwrap_or(false);
            let max_results = args["max_results"].as_u64().unwrap_or(1000) as usize;

            let mut builder = WalkBuilder::new(path);
            builder.hidden(!hidden)
                .git_ignore(true)
                .git_global(true)
                .git_exclude(true);

            if !recursive {
                builder.max_depth(Some(1));
            } else if let Some(d) = max_depth {
                builder.max_depth(Some(d));
            }

            // Glob/ext overrides
            if !pattern.is_empty() || !ext.is_empty() {
                let mut ov = OverrideBuilder::new(path);
                if !pattern.is_empty() {
                    let _ = ov.add(pattern);
                }
                if !ext.is_empty() {
                    let _ = ov.add(&format!("*.{ext}"));
                }
                if let Ok(overrides) = ov.build() {
                    builder.overrides(overrides);
                }
            }

            let mut results = Vec::new();
            for entry in builder.build().flatten() {
                let is_dir = entry.file_type().map_or(false, |ft| ft.is_dir());
                if dirs_only && !is_dir { continue; }
                if !dirs_only && is_dir { continue; }
                results.push(entry.path().display().to_string());
                if results.len() >= max_results { break; }
            }

            if results.is_empty() {
                "No files found.".into()
            } else {
                let total = results.len();
                let output = results.join("\n");
                if total >= max_results {
                    format!("{output}\n[limited to {max_results} results]")
                } else {
                    output
                }
            }
        }
        "search_text" => {
            use ignore::WalkBuilder;
            use ignore::overrides::OverrideBuilder;
            use grep::regex::RegexMatcherBuilder;
            use grep::searcher::{SearcherBuilder, sinks::UTF8};

            let pattern = args["pattern"].as_str().unwrap_or("");
            if pattern.is_empty() {
                return "Error: pattern is required".into();
            }
            let path = args["path"].as_str().unwrap_or(".");
            let ext = args["ext"].as_str().unwrap_or("");
            let glob = args["glob"].as_str().unwrap_or("");
            let case_insensitive = args["case_insensitive"].as_bool().unwrap_or(false);
            let hidden = args["hidden"].as_bool().unwrap_or(false);
            let max_results = args["max_results"].as_u64().unwrap_or(500) as usize;
            let context_lines = args["context_lines"].as_u64().unwrap_or(0) as usize;

            let matcher = match RegexMatcherBuilder::new()
                .case_insensitive(case_insensitive)
                .build(pattern)
            {
                Ok(m) => m,
                Err(e) => return format!("Error: invalid pattern: {e}"),
            };

            let mut walk_builder = WalkBuilder::new(path);
            walk_builder.hidden(!hidden)
                .git_ignore(true)
                .git_global(true)
                .git_exclude(true);

            if !ext.is_empty() || !glob.is_empty() {
                let mut ov = OverrideBuilder::new(path);
                if !ext.is_empty() {
                    let _ = ov.add(&format!("*.{ext}"));
                }
                if !glob.is_empty() {
                    let _ = ov.add(glob);
                }
                if let Ok(overrides) = ov.build() {
                    walk_builder.overrides(overrides);
                }
            }

            let mut results = Vec::new();
            let mut done = false;

            for entry in walk_builder.build().flatten() {
                if done { break; }
                if !entry.file_type().map_or(false, |ft| ft.is_file()) { continue; }
                let file_path = entry.path().to_path_buf();

                let mut searcher = SearcherBuilder::new()
                    .before_context(context_lines)
                    .after_context(context_lines)
                    .build();

                let _ = searcher.search_path(
                    &matcher,
                    &file_path,
                    UTF8(|line_num, line| {
                        results.push(format!("{}:{}:{}", file_path.display(), line_num, line.trim_end()));
                        if results.len() >= max_results {
                            done = true;
                            return Ok(false); // stop searching this file
                        }
                        Ok(true)
                    }),
                );
            }

            if results.is_empty() {
                "No matches found.".into()
            } else {
                let total = results.len();
                let output = results.join("\n");
                if total >= max_results {
                    format!("{output}\n[limited to {max_results} results]")
                } else {
                    output
                }
            }
        }
        _ => format!("Unknown tool: {name}"),
    }
}

// ── PlannerTools: read-only + create_plan ────────────────────────────────

pub struct PlannerTools {
    memory: Option<Arc<Memory>>,
    plan: std::sync::Mutex<Option<Vec<String>>>,
}

impl PlannerTools {
    pub fn new(memory: Option<Arc<Memory>>) -> Self {
        Self { memory, plan: std::sync::Mutex::new(None) }
    }

    /// Take the plan out (if create_plan was called).
    pub fn take_plan(&self) -> Option<Vec<String>> {
        self.plan.lock().unwrap().take()
    }
}

impl ToolExecutor for PlannerTools {
    fn definitions(&self) -> Vec<ToolDef> {
        vec![
            list_files_tool(),
            search_text_tool(),
            shell_tool(),
            memory_recall_tool(),
            create_plan_tool(),
        ]
    }

    async fn execute(&self, name: &str, args: &serde_json::Value) -> String {
        if name == "create_plan" {
            let steps: Vec<String> = args["steps"]
                .as_array()
                .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default();
            *self.plan.lock().unwrap() = Some(steps.clone());
            return format!("Plan created with {} steps", steps.len());
        }
        if !["memory_recall", "list_files", "search_text", "shell"].contains(&name) {
            return format!("Tool '{name}' not available in planning phase");
        }
        execute_tool(name, args, &self.memory).await
    }

    fn continuation_check(&self) -> Option<String> {
        if self.plan.lock().unwrap().is_none() {
            Some("[system] You responded with text instead of calling create_plan. Call create_plan now with the steps for the user's original request.".into())
        } else {
            None
        }
    }

    fn should_stop(&self) -> bool {
        self.plan.lock().unwrap().is_some()
    }
}

// ── AgentTools: full executor toolset ───────────────────────────────────

pub struct AgentTools {
    memory: Option<Arc<Memory>>,
}

impl AgentTools {
    pub fn new(memory: Option<Arc<Memory>>) -> Self {
        Self { memory }
    }
}

impl ToolExecutor for AgentTools {
    fn definitions(&self) -> Vec<ToolDef> {
        vec![
            list_files_tool(),
            search_text_tool(),
            file_read_tool(),
            file_write_tool(),
            send_file_tool(),
            shell_tool(),
            memory_recall_tool(),
        ]
    }

    async fn execute(&self, name: &str, args: &serde_json::Value) -> String {
        execute_tool(name, args, &self.memory).await
    }
}

// ── SupervisorTools: decision + memory ───────────────────────────────────

fn deliver_answer_tool() -> ToolDef {
    ToolDef {
        kind: "function".into(),
        function: FunctionDef {
            name: "deliver_answer".into(),
            description: "Deliver the final answer to the user. Call this when the request is fully addressed.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "answer": { "type": "string", "description": "The final answer to present to the user" }
                },
                "required": ["answer"]
            }),
        },
    }
}

fn request_more_work_tool() -> ToolDef {
    ToolDef {
        kind: "function".into(),
        function: FunctionDef {
            name: "request_more_work".into(),
            description: "Request more work from the planner. Call this when the task is fundamentally incomplete.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "feedback": { "type": "string", "description": "What is missing or incomplete and why. Be specific — this goes to the planner." }
                },
                "required": ["feedback"]
            }),
        },
    }
}

pub enum SupervisorDecision {
    Done(String),
    NeedsWork(String),
}

pub struct SupervisorTools {
    memory: Option<Arc<Memory>>,
    decision: std::sync::Mutex<Option<SupervisorDecision>>,
}

impl SupervisorTools {
    pub fn new(memory: Option<Arc<Memory>>) -> Self {
        Self { memory, decision: std::sync::Mutex::new(None) }
    }

    pub fn take_decision(&self) -> Option<SupervisorDecision> {
        self.decision.lock().unwrap().take()
    }
}

impl ToolExecutor for SupervisorTools {
    fn definitions(&self) -> Vec<ToolDef> {
        vec![
            deliver_answer_tool(),
            request_more_work_tool(),
            memory_store_tool(),
            memory_recall_tool(),
        ]
    }

    async fn execute(&self, name: &str, args: &serde_json::Value) -> String {
        match name {
            "deliver_answer" => {
                let answer = args["answer"].as_str().unwrap_or("").to_string();
                *self.decision.lock().unwrap() = Some(SupervisorDecision::Done(answer));
                "Answer delivered.".into()
            }
            "request_more_work" => {
                let feedback = args["feedback"].as_str().unwrap_or("").to_string();
                *self.decision.lock().unwrap() = Some(SupervisorDecision::NeedsWork(feedback));
                "Feedback sent to planner.".into()
            }
            "memory_store" | "memory_recall" => {
                execute_tool(name, args, &self.memory).await
            }
            _ => format!("Tool '{name}' not available in supervisor phase"),
        }
    }

    fn continuation_check(&self) -> Option<String> {
        if self.decision.lock().unwrap().is_none() {
            Some("[system] You must call either deliver_answer or request_more_work.".into())
        } else {
            None
        }
    }

    fn should_stop(&self) -> bool {
        self.decision.lock().unwrap().is_some()
    }
}
