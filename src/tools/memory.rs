use crate::memory::Memory;
use crate::react::{FunctionDef, ToolDef};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

// ── MemoryStoreTool ─────────────────────────────────────────────────────

pub struct MemoryStoreTool {
    pub memory: Option<Arc<Memory>>,
}

impl super::Tool for MemoryStoreTool {
    fn name(&self) -> &'static str {
        "memory_store"
    }

    fn definition(&self) -> ToolDef {
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

    fn execute<'a>(&'a self, args: &'a serde_json::Value) -> Pin<Box<dyn Future<Output = String> + Send + 'a>> {
        Box::pin(async move {
            let Some(ref mem) = self.memory else { return "Memory not available".into() };
            let key = args["key"].as_str().unwrap_or("");
            let content = args["content"].as_str().unwrap_or("");
            match mem.store(key, content).await {
                Ok(_) => format!("Stored memory: {key}"),
                Err(e) => format!("Error storing memory: {e}"),
            }
        })
    }
}

// ── MemoryRecallTool ────────────────────────────────────────────────────

pub struct MemoryRecallTool {
    pub memory: Option<Arc<Memory>>,
}

impl super::Tool for MemoryRecallTool {
    fn name(&self) -> &'static str {
        "memory_recall"
    }

    fn definition(&self) -> ToolDef {
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

    fn execute<'a>(&'a self, args: &'a serde_json::Value) -> Pin<Box<dyn Future<Output = String> + Send + 'a>> {
        Box::pin(async move {
            let Some(ref mem) = self.memory else { return "Memory not available".into() };
            let query = args["query"].as_str().unwrap_or("");
            match mem.recall(query, 5).await {
                Ok(entries) if entries.is_empty() => "No relevant memories found.".into(),
                Ok(entries) => entries.iter()
                    .map(|e| format!("- {} (score: {:.2}): {}", e.key, e.score, e.content))
                    .collect::<Vec<_>>()
                    .join("\n"),
                Err(e) => format!("Error recalling memory: {e}"),
            }
        })
    }
}
