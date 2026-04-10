use crate::config::Config;
use crate::memory::Memory;
use crate::react::{self, FunctionDef, Message, ReactConfig, ToolDef, ToolExecutor};
use crate::tools::Tool;
use crate::{emit, tools};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use super::handlers::InternalHandler;

// ── Prompt ──────────────────────────────────────────────────────────────

const PROMPT: &str = r#"
You are the planner. Your job: deeply understand the request, thoroughly investigate the context, then create a plan.

## Investigation
Your tools for research:
- list_files — find files by name, extension, or glob pattern. Use this to understand the project structure.
- search_text — search file contents by regex pattern. Use this to find which files contain relevant content.
- shell — run commands for context gathering (git log, checking environment, etc). Do NOT use it to read file contents.
- memory_recall — search long-term memory for things you already know.

You do NOT have file_read. Your job is to find what's relevant and plan, not to read file contents. The executor will read and act on the files.

This is the most important phase. Do NOT rush to create a plan. Investigate first.

- Start with memory_recall to check if you already know something relevant.
- Use list_files to explore the project structure. Use recursive mode and extension filters.
- Use search_text to find specific patterns or content across files.
- When the user says "all" or "every", you must exhaustively search. Don't stop at the first match.
- If a tool call reveals more to explore, keep going. Follow every lead.
- Only stop investigating when you are confident you have the full picture.
- If a tool call fails, try again with corrected arguments. Don't give up.

## Executor capabilities
The executor agent that will carry out your plan has access to all the tools and skills described in the system prompt above, including:
- shell (run any command), file_read, file_write, memory_store, memory_recall
- All skills (web search, image generation, integrations, etc.)

Plan with these capabilities in mind. If a skill can do something, use it.

## Creating the plan
When you fully understand the request and have gathered all context, call the `create_plan` tool.

Each step is a deliverable — a concrete result that a separate worker agent must produce. The worker knows NOTHING about your investigation. It only sees the step description.

Define what to deliver, not how to do it. Include your findings so the worker has context.

Format every step as:
"Context: <what the user wants + your relevant findings>. Deliverable: <what this worker should produce and present>"

Group related work. If multiple files contribute to one answer, that's one deliverable, not one step per file."#;

// ── CreatePlanTool ──────────────────────────────────────────────────────

struct CreatePlanTool {
    plan: Arc<Mutex<Option<Vec<String>>>>,
}

impl Tool for CreatePlanTool {
    fn name(&self) -> &'static str {
        "create_plan"
    }

    fn definition(&self) -> ToolDef {
        ToolDef {
            kind: "function".into(),
            function: FunctionDef {
                name: "create_plan".into(),
                description: "Submit the execution plan. Call this when you are ready with your plan.".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "steps": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Ordered list of steps for the executor to perform"
                        }
                    },
                    "required": ["steps"]
                }),
            },
        }
    }

    fn execute<'a>(&'a self, args: &'a serde_json::Value) -> Pin<Box<dyn Future<Output = String> + Send + 'a>> {
        Box::pin(async move {
            let steps: Vec<String> = args["steps"]
                .as_array()
                .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default();
            *self.plan.lock().unwrap() = Some(steps.clone());
            format!("Plan created with {} steps", steps.len())
        })
    }
}

// ── PlannerTools ────────────────────────────────────────────────────────

struct PlannerTools {
    tools: Vec<Box<dyn Tool>>,
    plan: Arc<Mutex<Option<Vec<String>>>>,
}

impl PlannerTools {
    fn new(mem: Option<Arc<Memory>>) -> Self {
        let plan: Arc<Mutex<Option<Vec<String>>>> = Arc::new(Mutex::new(None));
        Self {
            tools: vec![
                Box::new(tools::list_files::ListFilesTool),
                Box::new(tools::search_text::SearchTextTool),
                Box::new(tools::shell::ShellTool),
                Box::new(tools::memory::MemoryRecallTool { memory: mem }),
                Box::new(CreatePlanTool { plan: plan.clone() }),
            ],
            plan,
        }
    }

    fn take_plan(&self) -> Option<Vec<String>> {
        self.plan.lock().unwrap().take()
    }
}

impl ToolExecutor for PlannerTools {
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

    fn continuation_check(&self) -> Option<String> {
        if self.plan.lock().unwrap().is_none() {
            Some("[system] You responded with text instead of calling create_plan. Call create_plan now with the steps for the user's original request.".into())
        } else {
            None
        }
    }

    fn should_stop(&self) -> bool {
        self.plan.lock().unwrap().is_some()
    }
}

// ── Phase function ──────────────────────────────────────────────────────

pub async fn plan(
    config: &Config,
    user_msg: &str,
    memory_ctx: &str,
    session_history: Vec<Message>,
    mem: Option<Arc<Memory>>,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let actor = "planner";
    let system = if config.skills_prompt.is_empty() {
        PROMPT.to_string()
    } else {
        format!("{}\n\n## Available skills\nThe executor has access to the following skills:\n{}", PROMPT, config.skills_prompt)
    };

    emit::emit("phase.start", actor, serde_json::json!({
        "system_prompt_chars": system.len(),
        "user_message": user_msg,
    }));

    let mut messages = vec![Message::system(&system)];
    if !memory_ctx.is_empty() {
        messages.push(Message::system(&format!("<memory>\n{memory_ctx}</memory>")));
    }
    messages.extend(session_history);
    messages.push(Message::user(user_msg));

    let react_config = ReactConfig {
        llm_url: config.llm_url.clone(),
        api_key: config.api_key.clone(),
        model: config.model.clone(),
        max_turns: 20,
        reasoning_effort: config.planner_reasoning.clone(),
    };

    let planner_tools = PlannerTools::new(mem);

    let _result = react::run(
        &react_config,
        &planner_tools,
        messages,
        &InternalHandler { actor },
    ).await;

    let steps = planner_tools.take_plan()
        .filter(|s| !s.is_empty())
        .ok_or("Planner did not call create_plan")?;

    emit::emit("plan", actor, serde_json::json!({"steps": steps}));
    Ok(steps)
}
