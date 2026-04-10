use crate::react::{FunctionDef, ToolDef};
use std::future::Future;
use std::pin::Pin;
use std::process::Stdio;
use tokio::process::Command;

pub struct ShellTool;

impl super::Tool for ShellTool {
    fn name(&self) -> &'static str {
        "shell"
    }

    fn definition(&self) -> ToolDef {
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

    fn execute<'a>(&'a self, args: &'a serde_json::Value) -> Pin<Box<dyn Future<Output = String> + Send + 'a>> {
        Box::pin(async move {
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
        })
    }
}
