use crate::react::{FunctionDef, ToolDef};
use std::future::Future;
use std::pin::Pin;

pub struct FileWriteTool;

impl super::Tool for FileWriteTool {
    fn name(&self) -> &'static str {
        "file_write"
    }

    fn definition(&self) -> ToolDef {
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

    fn execute<'a>(&'a self, args: &'a serde_json::Value) -> Pin<Box<dyn Future<Output = String> + Send + 'a>> {
        Box::pin(async move {
            let path = args["path"].as_str().unwrap_or("");
            let content = args["content"].as_str().unwrap_or("");
            if let Some(p) = std::path::Path::new(path).parent() {
                let _ = std::fs::create_dir_all(p);
            }
            match std::fs::write(path, content) {
                Ok(_) => format!("Written to {path}"),
                Err(e) => format!("Error: {e}"),
            }
        })
    }
}
