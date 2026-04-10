use crate::config::Config;
use crate::memory::Memory;
use crate::react::{self, FunctionDef, Message, ReactConfig, ToolDef, ToolExecutor};
use crate::tools::Tool;
use crate::{emit, tools};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use super::handlers::InternalHandler;
use crate::agent::CompletedStep;

// ── Prompt ──────────────────────────────────────────────────────────────

const PROMPT: &str = r#"
You are the supervisor. Review the original request and what was accomplished, then make a decision.

## Analysis
Before deciding, carefully analyze ALL executor outputs together. Look for:
- References to files, resources, or information that were mentioned but not followed up on
- Discoveries that change the scope or approach of the task
- Gaps between what was asked and what was delivered
- New leads or insights that emerged during execution

Include any important findings in your feedback to the planner if requesting more work.

## Decision
You MUST call one of these tools:
- `deliver_answer` — when the request is fully addressed. Synthesize results into one clean answer for the user. Don't list step numbers.
- `request_more_work` — when the task is fundamentally incomplete. Include what's missing AND any new discoveries from executor outputs that the planner should know about.

You also have `memory_store` and `memory_recall`. Save any durable facts worth remembering from the USER's request — preferences, personal details, project context, decisions.
Do NOT save: your own outputs, tool results, transient requests, or things already in memory.

## Rules
- Be conservative. Only request more work when the task is fundamentally incomplete, not just suboptimal.
- If the executor already performed irreversible actions (sent files, emails, generated images, posted messages), accept what was done. You cannot see files or images — do not second-guess their quality."#;

// ── SupervisorDecision ──────────────────────────────────────────────────

pub enum SupervisorDecision {
    Done(String),
    NeedsWork(String),
}

// ── DeliverAnswerTool ───────────────────────────────────────────────────

struct DeliverAnswerTool {
    decision: Arc<Mutex<Option<SupervisorDecision>>>,
}

impl Tool for DeliverAnswerTool {
    fn name(&self) -> &'static str {
        "deliver_answer"
    }

    fn definition(&self) -> ToolDef {
        ToolDef {
            kind: "function".into(),
            function: FunctionDef {
                name: "deliver_answer".into(),
                description: "Deliver the final answer to the user. Call this when the request is fully addressed.".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "answer": { "type": "string", "description": "The final answer to present to the user" }
                    },
                    "required": ["answer"]
                }),
            },
        }
    }

    fn execute<'a>(&'a self, args: &'a serde_json::Value) -> Pin<Box<dyn Future<Output = String> + Send + 'a>> {
        Box::pin(async move {
            let answer = args["answer"].as_str().unwrap_or("").to_string();
            *self.decision.lock().unwrap() = Some(SupervisorDecision::Done(answer));
            "Answer delivered.".into()
        })
    }
}

// ── RequestMoreWorkTool ─────────────────────────────────────────────────

struct RequestMoreWorkTool {
    decision: Arc<Mutex<Option<SupervisorDecision>>>,
}

impl Tool for RequestMoreWorkTool {
    fn name(&self) -> &'static str {
        "request_more_work"
    }

    fn definition(&self) -> ToolDef {
        ToolDef {
            kind: "function".into(),
            function: FunctionDef {
                name: "request_more_work".into(),
                description: "Request more work from the planner. Call this when the task is fundamentally incomplete.".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "feedback": { "type": "string", "description": "What is missing or incomplete and why. Be specific — this goes to the planner." }
                    },
                    "required": ["feedback"]
                }),
            },
        }
    }

    fn execute<'a>(&'a self, args: &'a serde_json::Value) -> Pin<Box<dyn Future<Output = String> + Send + 'a>> {
        Box::pin(async move {
            let feedback = args["feedback"].as_str().unwrap_or("").to_string();
            *self.decision.lock().unwrap() = Some(SupervisorDecision::NeedsWork(feedback));
            "Feedback sent to planner.".into()
        })
    }
}

// ── SupervisorTools ─────────────────────────────────────────────────────

struct SupervisorTools {
    tools: Vec<Box<dyn Tool>>,
    decision: Arc<Mutex<Option<SupervisorDecision>>>,
}

impl SupervisorTools {
    fn new(mem: Option<Arc<Memory>>) -> Self {
        let decision: Arc<Mutex<Option<SupervisorDecision>>> = Arc::new(Mutex::new(None));
        Self {
            tools: vec![
                Box::new(DeliverAnswerTool { decision: decision.clone() }),
                Box::new(RequestMoreWorkTool { decision: decision.clone() }),
                Box::new(tools::memory::MemoryStoreTool { memory: mem.clone() }),
                Box::new(tools::memory::MemoryRecallTool { memory: mem }),
            ],
            decision,
        }
    }

    fn take_decision(&self) -> Option<SupervisorDecision> {
        self.decision.lock().unwrap().take()
    }
}

impl ToolExecutor for SupervisorTools {
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
        if self.decision.lock().unwrap().is_none() {
            Some("[system] You must call either deliver_answer or request_more_work.".into())
        } else {
            None
        }
    }

    fn should_stop(&self) -> bool {
        self.decision.lock().unwrap().is_some()
    }
}

// ── Phase function ──────────────────────────────────────────────────────

pub async fn supervise(
    config: &Config,
    request: &str,
    completed: &[CompletedStep],
    mem: Option<Arc<Memory>>,
) -> Result<SupervisorDecision, Box<dyn std::error::Error>> {
    let actor = "supervisor";
    let system = if config.soul_prompt.is_empty() {
        PROMPT.to_string()
    } else {
        format!("{}\n\n{}", config.soul_prompt, PROMPT)
    };
    let user = format!(
        "Original request: {}\n\nCompleted steps:\n{}\n\nIs the request fully answered, or are more steps needed?",
        request, crate::agent::format_completed(completed)
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
        max_turns: 5,
        reasoning_effort: config.supervisor_reasoning.clone(),
    };

    let supervisor_tools = SupervisorTools::new(mem);

    let _result = react::run(
        &react_config,
        &supervisor_tools,
        messages,
        &InternalHandler { actor },
    ).await;

    let decision = supervisor_tools.take_decision()
        .unwrap_or(SupervisorDecision::Done("No answer produced.".into()));

    match &decision {
        SupervisorDecision::Done(answer) => {
            emit::emit("supervise", actor, serde_json::json!({"action": "complete", "answer": answer}));
        }
        SupervisorDecision::NeedsWork(feedback) => {
            emit::emit("supervise", actor, serde_json::json!({"action": "needs_work", "feedback": feedback}));
        }
    }

    Ok(decision)
}
