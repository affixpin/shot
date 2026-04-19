use crate::config::Config;
use crate::emit;
use crate::react::{self, Message, ReactConfig, ReactHandler, Usage};
use crate::session::Session;
use crate::tools::ExternalTools;
use std::collections::HashMap;

// ── Event handler ──────────────────────────────────────────────────────

struct EventHandler;

impl ReactHandler for EventHandler {
    fn on_llm_request(&self, turn: usize, message_count: usize) {
        emit::emit("llm.request", serde_json::json!({"turn": turn, "messages": message_count}));
    }
    fn on_llm_response(&self, turn: usize, content: &str, tool_call_count: usize) {
        emit::emit("llm.response", serde_json::json!({"turn": turn, "content": content, "tool_calls": tool_call_count}));
    }
    fn on_llm_error(&self, turn: usize, error: &str) {
        emit::emit("llm.error", serde_json::json!({"turn": turn, "error": error}));
    }
    fn on_text(&self, text: &str) {
        emit::emit("text", serde_json::json!({"content": text}));
    }
    fn on_tool_call(&self, name: &str, args: &serde_json::Value) {
        emit::emit("tool.call", serde_json::json!({"name": name, "args": args}));
    }
    fn on_tool_result(&self, name: &str, result: &str) {
        emit::emit("tool.result", serde_json::json!({"name": name, "result": result}));
    }
    fn on_turn_complete(&self, turn: usize, message_count: usize, usage: &Usage) {
        emit::emit("turn.complete", serde_json::json!({
            "turn": turn, "messages": message_count,
            "total_tokens": usage.total_tokens,
            "prompt_tokens": usage.prompt_tokens,
            "completion_tokens": usage.completion_tokens,
        }));
    }
}

// ── Run options ────────────────────────────────────────────────────────

pub struct RunOptions<'a> {
    pub session_path: Option<&'a str>,
    pub message: &'a str,
    /// If Some, only load these tools. If None, load all from tools_dir.
    pub enabled_tools: Option<Vec<String>>,
    /// Per-tool var overrides from CLI flags.
    pub tool_overrides: HashMap<String, HashMap<String, String>>,
    /// Tools the agent MUST call. Marked as REQUIRED in the system prompt.
    pub required_tools: Vec<String>,
    /// Replace the soul (base personality). If None, use SOUL.md from config.
    pub soul_override: Option<String>,
    /// Append additional instructions to the soul.
    pub prompt_addition: Option<String>,
    /// Activated skills — contents of `<skills_dir>/<name>.md` files, in
    /// order, appended to the system prompt between soul and prompt_addition.
    pub skills: Vec<String>,
}

// ── Run ────────────────────────────────────────────────────────────────

pub async fn run(
    config: &Config,
    opts: RunOptions<'_>,
) -> Result<String, Box<dyn std::error::Error>> {
    let session = opts.session_path.map(|p| {
        Session::open(p, 200_000).expect("Failed to open session")
    });
    let session_history = session.as_ref()
        .map(|s| s.recent())
        .unwrap_or_default();

    let tools = ExternalTools::load(
        &config.tools_dir,
        opts.enabled_tools.as_deref(),
        &opts.tool_overrides,
    );
    let handler = EventHandler;

    // System prompt = soul + skills + prompt_addition
    let mut system = opts.soul_override.unwrap_or_else(|| config.soul_prompt.clone());
    for skill in &opts.skills {
        if !system.is_empty() { system.push_str("\n\n"); }
        system.push_str(skill);
    }
    if let Some(addition) = opts.prompt_addition {
        if !system.is_empty() { system.push_str("\n\n"); }
        system.push_str(&addition);
    }

    // Append dynamic tool list
    let tool_descs = tools.descriptions();
    if tool_descs.is_empty() {
        if !system.is_empty() { system.push_str("\n\n"); }
        system.push_str("You currently have NO tools available. Answer directly from your knowledge. Do NOT claim to have any tools, do NOT pretend to call any. If you cannot answer something, say so honestly.");
    } else {
        if !system.is_empty() { system.push_str("\n\n"); }
        system.push_str("## Available tools\n\n");
        for (name, desc) in &tool_descs {
            if opts.required_tools.contains(name) {
                system.push_str(&format!("- `{name}` **(REQUIRED — you MUST call this)**: {desc}\n"));
            } else {
                system.push_str(&format!("- `{name}`: {desc}\n"));
            }
        }
        if !opts.required_tools.is_empty() {
            system.push_str(&format!(
                "\n**IMPORTANT:** You must call the following tool(s) as part of completing this task: {}. They are the primary mechanism by which your response is delivered — answering without calling them produces an incomplete result.\n",
                opts.required_tools.iter().map(|t| format!("`{t}`")).collect::<Vec<_>>().join(", ")
            ));
        }
    }

    let react_config = ReactConfig {
        llm_url: config.llm_url.clone(),
        api_key: config.api_key.clone(),
        model: config.model.clone(),
        max_turns: config.max_turns,
        reasoning_effort: config.reasoning.clone(),
    };

    emit::emit("user.message", serde_json::json!({"content": opts.message}));

    let mut messages = vec![Message::system(&system)];
    let session_len = session_history.len();
    messages.extend(session_history);
    messages.push(Message::user(opts.message));

    let result = react::run(&react_config, &tools, messages, &handler).await?;

    if let Some(ref s) = session {
        let new_start = 1 + session_len;
        for msg in result.messages.iter().skip(new_start) {
            s.push(msg);
        }
    }

    emit::emit("done", serde_json::json!({"total_tokens": result.total_tokens}));
    Ok(result.response)
}
