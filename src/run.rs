use crate::config::Config;
use crate::emit;
use crate::react::{self, Message, ReactConfig, ReactHandler};
use crate::session::Session;
use crate::tools::ExternalTools;

// ── Event handler ──────────────────────────────────────────────────────

struct EventHandler {
    actor: String,
    color: String,
}

impl ReactHandler for EventHandler {
    fn on_llm_request(&self, turn: usize, message_count: usize) {
        emit::emit("llm.request", &self.actor, &self.color, serde_json::json!({"turn": turn, "messages": message_count}));
    }
    fn on_llm_response(&self, turn: usize, content: &str, tool_call_count: usize) {
        emit::emit("llm.response", &self.actor, &self.color, serde_json::json!({"turn": turn, "content": content, "tool_calls": tool_call_count}));
    }
    fn on_llm_error(&self, turn: usize, error: &str) {
        emit::emit("llm.error", &self.actor, &self.color, serde_json::json!({"turn": turn, "error": error}));
    }
    fn on_text(&self, text: &str) {
        emit::emit("text", &self.actor, &self.color, serde_json::json!({"content": text}));
    }
    fn on_tool_call(&self, name: &str, args: &serde_json::Value) {
        emit::emit("tool.call", &self.actor, &self.color, serde_json::json!({"name": name, "args": args}));
    }
    fn on_tool_result(&self, name: &str, result: &str) {
        let truncated = if result.len() > 2000 { &result[..2000] } else { result };
        emit::emit("tool.result", &self.actor, &self.color, serde_json::json!({"name": name, "result": truncated}));
    }
}

// ── Run ────────────────────────────────────────────────────────────────

pub async fn run(
    config: &Config,
    role_name: Option<&str>,
    session_path: Option<&str>,
    context: &str,
    message: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let actor = role_name.unwrap_or("shot");

    // Resolve role from config
    let role = match role_name {
        Some(name) => config.roles.get(name)
            .ok_or_else(|| format!("Unknown role: {name}"))?.clone(),
        None => config.roles.get("default")
            .ok_or("No 'default' role in config. Add [agent.roles.default] to agent.toml")?.clone(),
    };

    // Load session
    let session = session_path.map(|p| {
        Session::open(p, 200_000).expect("Failed to open session")
    });
    let session_history = session.as_ref()
        .map(|s| s.recent())
        .unwrap_or_default();

    let tools = ExternalTools::load(&config.tools_dir, &role.tools);
    let handler = EventHandler { actor: actor.into(), color: role.color.clone() };

    // Build system prompt: soul + role prompt
    let mut system = String::new();
    if !config.soul_prompt.is_empty() {
        system.push_str(&config.soul_prompt);
        system.push_str("\n\n");
    }
    system.push_str(&role.prompt);

    let react_config = ReactConfig {
        llm_url: config.llm_url.clone(),
        api_key: config.api_key.clone(),
        model: config.model.clone(),
        max_turns: config.max_turns,
        reasoning_effort: config.reasoning.clone(),
    };

    let mut messages = vec![Message::system(&system)];
    let session_len = session_history.len();
    messages.extend(session_history);
    if !context.is_empty() {
        messages.push(Message::user(format!("<context>\n{context}\n</context>")));
    }
    messages.push(Message::user(message));

    let result = react::run(&react_config, &tools, messages, &handler).await?;

    // Save new messages (after system + session history) to session
    if let Some(ref s) = session {
        // Skip: system prompt (1) + session history (session_len)
        let new_start = 1 + session_len;
        for msg in result.messages.iter().skip(new_start) {
            s.push(msg);
        }
    }

    emit::emit("done", actor, &role.color, serde_json::json!({}));
    Ok(result.response)
}
