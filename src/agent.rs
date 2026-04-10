use crate::{config::Config, emit, memory::Memory, react::Message, roles, session::Session};
use std::sync::Arc;

use roles::supervisor::SupervisorDecision;

// ── Helpers ─────────────────────────────────────────────────────────────

pub struct CompletedStep {
    pub step: String,
    pub result: String,
}

pub(crate) fn format_completed(completed: &[CompletedStep]) -> String {
    if completed.is_empty() {
        return "(none)".into();
    }
    completed.iter()
        .map(|c| format!("- Step: {}\n  Result: {}", c.step, c.result))
        .collect::<Vec<_>>()
        .join("\n")
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
        let mut steps = roles::planner::plan(config, &planner_context, &memory_ctx, session_history.clone(), mem.clone()).await?;
        let total_steps = steps.len();

        // Execute all steps
        while !steps.is_empty() {
            let current_step = steps.remove(0);
            step_index += 1;

            emit::emit("step.start", "executor", serde_json::json!({
                "index": step_index, "total": total_steps, "description": current_step,
            }));

            let result = roles::executor::execute_step(config, user_msg, &current_step, &completed, mem.clone()).await?;

            emit::emit("step.end", "executor", serde_json::json!({
                "index": step_index, "result": result,
            }));

            completed.push(CompletedStep { step: current_step, result });
        }

        // Supervise
        match roles::supervisor::supervise(config, user_msg, &completed, mem.clone()).await? {
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
