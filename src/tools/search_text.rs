use crate::react::{FunctionDef, ToolDef};
use grep::regex::RegexMatcherBuilder;
use grep::searcher::{SearcherBuilder, sinks::UTF8};
use ignore::WalkBuilder;
use ignore::overrides::OverrideBuilder;
use std::future::Future;
use std::pin::Pin;

pub struct SearchTextTool;

impl super::Tool for SearchTextTool {
    fn name(&self) -> &'static str {
        "search_text"
    }

    fn definition(&self) -> ToolDef {
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

    fn execute<'a>(&'a self, args: &'a serde_json::Value) -> Pin<Box<dyn Future<Output = String> + Send + 'a>> {
        Box::pin(async move {
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
                            return Ok(false);
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
        })
    }
}
