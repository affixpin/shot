use crate::react::{FunctionDef, ToolDef};
use std::future::Future;
use std::pin::Pin;

pub struct SendFileTool;

impl super::Tool for SendFileTool {
    fn name(&self) -> &'static str {
        "send_file"
    }

    fn definition(&self) -> ToolDef {
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

    fn execute<'a>(&'a self, args: &'a serde_json::Value) -> Pin<Box<dyn Future<Output = String> + Send + 'a>> {
        Box::pin(async move {
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
        })
    }
}
