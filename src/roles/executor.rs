use crate::config::Config;
use crate::memory::Memory;
use crate::react::{self, Message, ReactConfig, ToolDef, ToolExecutor};
use crate::tools::Tool;
use crate::{emit, tools};
use std::sync::Arc;

use super::handlers::StreamHandler;
use crate::agent::CompletedStep;

// ── Prompt ──────────────────────────────────────────────────────────────

const PROMPT: &str = r#"
You are a worker producing a deliverable. Your step describes what to deliver and gives you context. Your job is to produce a high-quality result.

## Approach
1. Read the step carefully — understand what deliverable is expected.
2. Use your tools to gather what you need. Act, don't guess.
3. If something fails, diagnose and retry. Don't give up on first failure.
4. Produce the deliverable. Present your findings clearly and completely.

## Rules
- Your output is your deliverable. Make it useful — don't just dump raw data, analyze and present it.
- When writing files, read the existing file first to understand context.
- The user CANNOT see files in your workspace. If the deliverable is a file, use send_file to deliver it.
- If the step includes context from the planner's investigation, use it — don't re-investigate what's already known.

## Error handling
When a tool returns an error, do NOT give up. Diagnose the problem, fix it, and retry. Only report failure after you've tried to fix it. Include the exact error message."#;

// ── AgentTools ──────────────────────────────────────────────────────────

struct AgentTools {
    tools: Vec<Box<dyn Tool>>,
}

impl AgentTools {
    fn new(mem: Option<Arc<Memory>>) -> Self {
        Self {
            tools: vec![
                Box::new(tools::list_files::ListFilesTool),
                Box::new(tools::search_text::SearchTextTool),
                Box::new(tools::file_read::FileReadTool),
                Box::new(tools::file_write::FileWriteTool),
                Box::new(tools::send_file::SendFileTool),
                Box::new(tools::shell::ShellTool),
                Box::new(tools::memory::MemoryRecallTool { memory: mem }),
            ],
        }
    }
}

impl ToolExecutor for AgentTools {
    fn definitions(&self) -> Vec<ToolDef> {
        self.tools.iter().map(|t| t.definition()).collect()
    }

    async fn execute(&self, name: &str, args: &serde_json::Value) -> String {
        for tool in &self.tools {
            if tool.name() == name {
                return tool.execute(args).await;
            }
        }
        format!("Unknown tool: {name}")
    }
}

// ── Phase function ──────────────────────────────────────────────────────

pub async fn execute_step(
    config: &Config,
    request: &str,
    step: &str,
    completed: &[CompletedStep],
    mem: Option<Arc<Memory>>,
) -> Result<String, Box<dyn std::error::Error>> {
    let actor = "executor";
    let system = if config.skills_prompt.is_empty() {
        PROMPT.to_string()
    } else {
        format!("{}\n\n## Skills\n{}", PROMPT, config.skills_prompt)
    };
    let user = format!(
        "Original request: {}\n\nYour task: {}\n\nCompleted so far:\n{}",
        request, step, crate::agent::format_completed(completed)
    );

    emit::emit("phase.start", actor, serde_json::json!({
        "system_prompt_chars": system.len(),
        "user_message": user,
    }));

    let messages = vec![
        Message::system(&system),
        Message::user(&user),
    ];

    let react_config = ReactConfig {
        llm_url: config.llm_url.clone(),
        api_key: config.api_key.clone(),
        model: config.model.clone(),
        max_turns: config.max_turns,
        reasoning_effort: config.executor_reasoning.clone(),
    };

    let result = react::run(
        &react_config,
        &AgentTools::new(mem),
        messages,
        &StreamHandler { actor },
    ).await?;

    emit::emit("phase.end", actor, serde_json::json!({"response": result.response}));
    Ok(result.response)
}
