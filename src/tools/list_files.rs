use crate::react::{FunctionDef, ToolDef};
use ignore::WalkBuilder;
use ignore::overrides::OverrideBuilder;
use std::future::Future;
use std::pin::Pin;

pub struct ListFilesTool;

impl super::Tool for ListFilesTool {
    fn name(&self) -> &'static str {
        "list_files"
    }

    fn definition(&self) -> ToolDef {
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

    fn execute<'a>(&'a self, args: &'a serde_json::Value) -> Pin<Box<dyn Future<Output = String> + Send + 'a>> {
        Box::pin(async move {
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
        })
    }
}
