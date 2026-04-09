use crate::{config::Config, emit, memory::Memory, prompts, react::{self, Message, ReactConfig, ReactHandler}, session::Session, tools::{AgentTools, SupervisorDecision, SupervisorTools, PlannerTools}};
use std::sync::Arc;

// ── Handlers ────────────────────────────────────────────────────────────

fn tool_detail(name: &str, args: &serde_json::Value) -> String {
    match name {
        "shell" => args["command"].as_str().unwrap_or("").to_string(),
        "file_read" => args["path"].as_str().unwrap_or("").to_string(),
        "file_write" => args["path"].as_str().unwrap_or("").to_string(),
        "list_files" => {
            let path = args["path"].as_str().unwrap_or(".");
            let ext = args["ext"].as_str().unwrap_or("");
            if ext.is_empty() { path.to_string() } else { format!("{path} *.{ext}") }
        }
        "search_text" => args["pattern"].as_str().unwrap_or("").to_string(),
        "memory_store" => args["key"].as_str().unwrap_or("").to_string(),
        "memory_recall" => args["query"].as_str().unwrap_or("").to_string(),
        "create_plan" => format!("{} steps", args["steps"].as_array().map(|a| a.len()).unwrap_or(0)),
        _ => String::new(),
    }
}

/// Silent handler — emits debug events only (for planner/supervisor).
struct InternalHandler { actor: &'static str }

impl ReactHandler for InternalHandler {
    fn on_llm_request(&self, turn: usize, message_count: usize) {
        emit::emit("llm.request", self.actor, serde_json::json!({"turn": turn, "messages": message_count}));
    }
    fn on_llm_response(&self, turn: usize, content: &str, tool_call_count: usize) {
        emit::emit("llm.response", self.actor, serde_json::json!({"turn": turn, "content": content, "tool_calls": tool_call_count}));
    }
    fn on_llm_error(&self, turn: usize, error: &str) {
        emit::emit("llm.error", self.actor, serde_json::json!({"turn": turn, "error": error}));
    }
    fn on_tool_call(&self, name: &str, args: &serde_json::Value) {
        emit::emit("tool.call", self.actor, serde_json::json!({"name": name, "args": args, "detail": tool_detail(name, args)}));
    }
    fn on_tool_result(&self, name: &str, result: &str) {
        let truncated = if result.len() > 2000 { &result[..2000] } else { result };
        emit::emit("tool.result", self.actor, serde_json::json!({"name": name, "result": truncated}));
    }
}

/// Streaming handler — emits user-facing text + debug events (for executor).
struct StreamHandler { actor: &'static str }

impl ReactHandler for StreamHandler {
    fn on_llm_request(&self, turn: usize, message_count: usize) {
        emit::emit("llm.request", self.actor, serde_json::json!({"turn": turn, "messages": message_count}));
    }
    fn on_llm_response(&self, turn: usize, content: &str, tool_call_count: usize) {
        emit::emit("llm.response", self.actor, serde_json::json!({"turn": turn, "content": content, "tool_calls": tool_call_count}));
    }
    fn on_llm_error(&self, turn: usize, error: &str) {
        emit::emit("llm.error", self.actor, serde_json::json!({"turn": turn, "error": error}));
    }
    fn on_text(&self, text: &str) {
        emit::emit("text", self.actor, serde_json::json!({"content": text}));
    }
    fn on_tool_call(&self, name: &str, args: &serde_json::Value) {
        emit::emit("tool.call", self.actor, serde_json::json!({"name": name, "args": args, "detail": tool_detail(name, args)}));
    }
    fn on_tool_result(&self, name: &str, result: &str) {
        let truncated = if result.len() > 2000 { &result[..2000] } else { result };
        emit::emit("tool.result", self.actor, serde_json::json!({"name": name, "result": truncated}));
    }
}


// ── Helpers ─────────────────────────────────────────────────────────────

pub struct CompletedStep {
    pub step: String,
    pub result: String,
}

fn format_completed(completed: &[CompletedStep]) -> String {
    if completed.is_empty() {
        return "(none)".into();
    }
    completed.iter()
        .map(|c| format!("- Step: {}\n  Result: {}", c.step, c.result))
        .collect::<Vec<_>>()
        .join("\n")
}

// ── Phase functions ─────────────────────────────────────────────────────

pub async fn plan(
    config: &Config,
    user_msg: &str,
    memory_ctx: &str,
    session_history: Vec<Message>,
    mem: Option<Arc<Memory>>,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let actor = "planner";
    let system = if config.skills_prompt.is_empty() {
        prompts::PLANNER.to_string()
    } else {
        format!("{}\n\n## Available skills\nThe executor has access to the following skills:\n{}", prompts::PLANNER, config.skills_prompt)
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
        actor,
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

pub async fn execute_step(
    config: &Config,
    request: &str,
    step: &str,
    completed: &[CompletedStep],
    mem: Option<Arc<Memory>>,
) -> Result<String, Box<dyn std::error::Error>> {
    let actor = "executor";
    let system = if config.skills_prompt.is_empty() {
        prompts::EXECUTOR.to_string()
    } else {
        format!("{}\n\n## Skills\n{}", prompts::EXECUTOR, config.skills_prompt)
    };
    let user = format!(
        "Original request: {}\n\nYour task: {}\n\nCompleted so far:\n{}",
        request, step, format_completed(completed)
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
        actor,
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

pub async fn supervise(
    config: &Config,
    request: &str,
    completed: &[CompletedStep],
    mem: Option<Arc<Memory>>,
) -> Result<SupervisorDecision, Box<dyn std::error::Error>> {
    let actor = "supervisor";
    let system = if config.soul_prompt.is_empty() {
        prompts::SUPERVISOR.to_string()
    } else {
        format!("{}\n\n{}", config.soul_prompt, prompts::SUPERVISOR)
    };
    let user = format!(
        "Original request: {}\n\nCompleted steps:\n{}\n\nIs the request fully answered, or are more steps needed?",
        request, format_completed(completed)
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
        actor,
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

// ── Orchestrator ────────────────────────────────────────────────────────

pub async fn run(config: &Config, user_msg: &str) -> Result<String, Box<dyn std::error::Error>> {
    let mem = Memory::open(&config.memory_db, &config.api_key, &config.embed_url)
        .map(Arc::new)
        .ok();

    let session = Session::open(&config.session_path, config.max_session_chars).ok();

    // Recall relevant memories
    let memory_ctx = match &mem {
        Some(m) => m.context_for(user_msg).await,
        None => String::new(),
    };
    let mem_hits = memory_ctx.lines().filter(|l| l.starts_with("- ")).count();

    if let Some(ref s) = session {
        s.push(&Message::user(user_msg));
    }

    let session_history = session.as_ref()
        .map(|s| s.recent())
        .unwrap_or_default();

    emit::emit_system("context", serde_json::json!({
        "memory_hits": mem_hits,
        "session_messages": session_history.len(),
    }));

    // ── Main loop: plan → execute → supervise → (repeat or done)
    let mut planner_context = user_msg.to_string();
    let mut completed: Vec<CompletedStep> = vec![];
    let mut step_index = 0usize;

    loop {
        // Plan
        let mut steps = plan(config, &planner_context, &memory_ctx, session_history.clone(), mem.clone()).await?;
        let total_steps = steps.len();

        // Execute all steps
        while !steps.is_empty() {
            let current_step = steps.remove(0);
            step_index += 1;

            emit::emit("step.start", "executor", serde_json::json!({
                "index": step_index, "total": total_steps, "description": current_step,
            }));

            let result = execute_step(config, user_msg, &current_step, &completed, mem.clone()).await?;

            emit::emit("step.end", "executor", serde_json::json!({
                "index": step_index, "result": result,
            }));

            completed.push(CompletedStep { step: current_step, result });
        }

        // Supervise
        match supervise(config, user_msg, &completed, mem.clone()).await? {
            SupervisorDecision::NeedsWork(feedback) => {
                // Feed supervisor's feedback back to planner as new context
                planner_context = format!(
                    "{}\n\nPrevious work was insufficient. Supervisor feedback: {}\n\nCompleted so far:\n{}",
                    user_msg, feedback, format_completed(&completed)
                );
                continue;
            }
            SupervisorDecision::Done(answer) => {
                if let Some(ref s) = session {
                    let msg = Message {
                        role: "assistant".into(),
                        content: Some(answer.clone()),
                        tool_calls: None, tool_call_id: None, extra: Default::default(),
                    };
                    s.push(&msg);
                }

                emit::emit("done", "supervisor", serde_json::json!({"content": answer}));
                return Ok(answer);
            }
        }
    }
}
