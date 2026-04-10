use crate::{emit, react::ReactHandler};

pub(crate) fn tool_detail(name: &str, args: &serde_json::Value) -> String {
    match name {
        "shell" => args["command"].as_str().unwrap_or("").to_string(),
        "file_read" => args["path"].as_str().unwrap_or("").to_string(),
        "file_write" => args["path"].as_str().unwrap_or("").to_string(),
        "list_files" => {
            let path = args["path"].as_str().unwrap_or(".");
            let ext = args["ext"].as_str().unwrap_or("");
            if ext.is_empty() { path.to_string() } else { format!("{path} *.{ext}") }
        }
        "search_text" => args["pattern"].as_str().unwrap_or("").to_string(),
        "memory_store" => args["key"].as_str().unwrap_or("").to_string(),
        "memory_recall" => args["query"].as_str().unwrap_or("").to_string(),
        "create_plan" => format!("{} steps", args["steps"].as_array().map(|a| a.len()).unwrap_or(0)),
        _ => String::new(),
    }
}

/// Silent handler — emits debug events only (for planner/supervisor).
pub struct InternalHandler {
    pub actor: &'static str,
}

impl ReactHandler for InternalHandler {
    fn on_llm_request(&self, turn: usize, message_count: usize) {
        emit::emit("llm.request", self.actor, serde_json::json!({"turn": turn, "messages": message_count}));
    }
    fn on_llm_response(&self, turn: usize, content: &str, tool_call_count: usize) {
        emit::emit("llm.response", self.actor, serde_json::json!({"turn": turn, "content": content, "tool_calls": tool_call_count}));
    }
    fn on_llm_error(&self, turn: usize, error: &str) {
        emit::emit("llm.error", self.actor, serde_json::json!({"turn": turn, "error": error}));
    }
    fn on_tool_call(&self, name: &str, args: &serde_json::Value) {
        emit::emit("tool.call", self.actor, serde_json::json!({"name": name, "args": args, "detail": tool_detail(name, args)}));
    }
    fn on_tool_result(&self, name: &str, result: &str) {
        let truncated = if result.len() > 2000 { &result[..2000] } else { result };
        emit::emit("tool.result", self.actor, serde_json::json!({"name": name, "result": truncated}));
    }
}

/// Streaming handler — emits user-facing text + debug events (for executor).
pub struct StreamHandler {
    pub actor: &'static str,
}

impl ReactHandler for StreamHandler {
    fn on_llm_request(&self, turn: usize, message_count: usize) {
        emit::emit("llm.request", self.actor, serde_json::json!({"turn": turn, "messages": message_count}));
    }
    fn on_llm_response(&self, turn: usize, content: &str, tool_call_count: usize) {
        emit::emit("llm.response", self.actor, serde_json::json!({"turn": turn, "content": content, "tool_calls": tool_call_count}));
    }
    fn on_llm_error(&self, turn: usize, error: &str) {
        emit::emit("llm.error", self.actor, serde_json::json!({"turn": turn, "error": error}));
    }
    fn on_text(&self, text: &str) {
        emit::emit("text", self.actor, serde_json::json!({"content": text}));
    }
    fn on_tool_call(&self, name: &str, args: &serde_json::Value) {
        emit::emit("tool.call", self.actor, serde_json::json!({"name": name, "args": args, "detail": tool_detail(name, args)}));
    }
    fn on_tool_result(&self, name: &str, result: &str) {
        let truncated = if result.len() > 2000 { &result[..2000] } else { result };
        emit::emit("tool.result", self.actor, serde_json::json!({"name": name, "result": truncated}));
    }
}
