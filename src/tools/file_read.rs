use crate::react::{FunctionDef, ToolDef};
use std::future::Future;
use std::pin::Pin;

pub struct FileReadTool;

impl super::Tool for FileReadTool {
    fn name(&self) -> &'static str {
        "file_read"
    }

    fn definition(&self) -> ToolDef {
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

    fn execute<'a>(&'a self, args: &'a serde_json::Value) -> Pin<Box<dyn Future<Output = String> + Send + 'a>> {
        Box::pin(async move {
            let path = args["path"].as_str().unwrap_or("");
            std::fs::read_to_string(path).unwrap_or_else(|e| format!("Error: {e}"))
        })
    }
}
