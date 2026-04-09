use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static PRETTY: AtomicBool = AtomicBool::new(false);

pub fn set_pretty(enabled: bool) {
    PRETTY.store(enabled, Ordering::Relaxed);
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

fn emit_raw(event: &serde_json::Value) {
    if PRETTY.load(Ordering::Relaxed) {
        pretty_print(event);
    } else {
        let mut stdout = std::io::stdout().lock();
        let _ = serde_json::to_writer(&mut stdout, event);
        let _ = stdout.write_all(b"\n");
        let _ = stdout.flush();
    }
}

/// Emit an event with the standard envelope: type, ts, actor, data.
pub fn emit(typ: &str, actor: &str, data: serde_json::Value) {
    emit_raw(&serde_json::json!({
        "type": typ,
        "ts": now(),
        "actor": actor,
        "data": data,
    }));
}

/// Emit an event without an actor (system-level events).
pub fn emit_system(typ: &str, data: serde_json::Value) {
    emit_raw(&serde_json::json!({
        "type": typ,
        "ts": now(),
        "data": data,
    }));
}

// ── Pretty printer ──────────────────────────────────────────────────────

// ANSI colors
const RESET: &str = "\x1b[0m";
const DIM: &str = "\x1b[2m";
const BOLD: &str = "\x1b[1m";
const RED: &str = "\x1b[31m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const BLUE: &str = "\x1b[34m";
const MAGENTA: &str = "\x1b[35m";
const CYAN: &str = "\x1b[36m";
const WHITE: &str = "\x1b[37m";

fn actor_color(actor: &str) -> &'static str {
    match actor {
        "planner" => CYAN,
        "executor" => GREEN,
        "supervisor" => MAGENTA,
        _ => WHITE,
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max { s.to_string() } else { format!("{}…", &s[..max]) }
}

fn pretty_print(event: &serde_json::Value) {
    let mut out = std::io::stderr().lock();
    let typ = event["type"].as_str().unwrap_or("?");
    let actor = event["actor"].as_str().unwrap_or("");
    let data = &event["data"];
    let color = actor_color(actor);

    let tag = if actor.is_empty() {
        format!("{DIM}[system]{RESET}")
    } else {
        format!("{color}{BOLD}{actor}{RESET}")
    };

    match typ {
        "phase.start" => {
            let msg = data["user_message"].as_str().unwrap_or("");
            let chars = data["system_prompt_chars"].as_u64().unwrap_or(0);
            let _ = writeln!(out, "\n{tag} {BOLD}▸ started{RESET} {DIM}(prompt: {chars} chars){RESET}");
            if !msg.is_empty() {
                let _ = writeln!(out, "  {DIM}{}{RESET}", truncate(msg, 120));
            }
        }
        "phase.end" => {
            let resp = data["response"].as_str().unwrap_or("");
            let _ = writeln!(out, "{tag} {DIM}▸ ended{RESET}");
            if !resp.is_empty() {
                let _ = writeln!(out, "  {DIM}{}{RESET}", truncate(resp, 120));
            }
        }
        "llm.request" => {
            let turn = data["turn"].as_u64().unwrap_or(0);
            let msgs = data["messages"].as_u64().unwrap_or(0);
            let _ = writeln!(out, "{tag} {DIM}← llm turn={turn} msgs={msgs}{RESET}");
        }
        "llm.response" => {
            let turn = data["turn"].as_u64().unwrap_or(0);
            let tc = data["tool_calls"].as_u64().unwrap_or(0);
            let content = data["content"].as_str().unwrap_or("");
            if tc > 0 {
                let _ = writeln!(out, "{tag} {DIM}→ llm turn={turn} tool_calls={tc}{RESET}");
            } else if !content.is_empty() {
                let _ = writeln!(out, "{tag} {DIM}→ llm turn={turn}{RESET} {}", truncate(content, 100));
            } else {
                let _ = writeln!(out, "{tag} {DIM}→ llm turn={turn} (empty){RESET}");
            }
        }
        "llm.error" => {
            let err = data["error"].as_str().unwrap_or("?");
            let _ = writeln!(out, "{tag} {RED}✗ llm error:{RESET} {}", truncate(err, 150));
        }
        "tool.call" => {
            let name = data["name"].as_str().unwrap_or("?");
            let args = &data["args"];
            let args_str = match name {
                "shell" => args["command"].as_str().unwrap_or("").to_string(),
                "file_read" => args["path"].as_str().unwrap_or("").to_string(),
                "file_write" => args["path"].as_str().unwrap_or("").to_string(),
                "list_files" => {
                    let path = args["path"].as_str().unwrap_or(".");
                    let ext = args["ext"].as_str().unwrap_or("");
                    let recursive = args["recursive"].as_bool().unwrap_or(false);
                    format!("{}{}{}", path, if recursive { " -R" } else { "" }, if ext.is_empty() { String::new() } else { format!(" *.{ext}") })
                }
                "search_text" => args["pattern"].as_str().unwrap_or("").to_string(),
                "create_plan" => format!("{} steps", args["steps"].as_array().map(|a| a.len()).unwrap_or(0)),
                "memory_recall" => args["query"].as_str().unwrap_or("").to_string(),
                "memory_store" => args["key"].as_str().unwrap_or("").to_string(),
                _ => serde_json::to_string(args).unwrap_or_default(),
            };
            let _ = writeln!(out, "{tag} {YELLOW}⚡ {name}{RESET} {}", truncate(&args_str, 100));
        }
        "tool.result" => {
            let name = data["name"].as_str().unwrap_or("?");
            let result = data["result"].as_str().unwrap_or("");
            let lines: Vec<&str> = result.lines().collect();
            if lines.len() <= 3 {
                let _ = writeln!(out, "{tag} {DIM}  ↳ {name}:{RESET} {}", truncate(result, 120));
            } else {
                let _ = writeln!(out, "{tag} {DIM}  ↳ {name}: ({} lines){RESET}", lines.len());
                for line in lines.iter().take(5) {
                    let _ = writeln!(out, "  {DIM}  {}{RESET}", truncate(line, 100));
                }
                if lines.len() > 5 {
                    let _ = writeln!(out, "  {DIM}  ... ({} more){RESET}", lines.len() - 5);
                }
            }
        }
        "tool.status" => {}
        "file" => {
            let path = data["path"].as_str().unwrap_or("?");
            let caption = data["caption"].as_str().unwrap_or("");
            let _ = writeln!(out, "{DIM}📎 sending: {path}{}{RESET}", if caption.is_empty() { String::new() } else { format!(" — {caption}") });
        }
        "plan" => {
            let steps = data["steps"].as_array();
            if let Some(steps) = steps {
                let _ = writeln!(out, "{tag} {BLUE}{BOLD}📋 Plan ({} steps){RESET}", steps.len());
                for (i, step) in steps.iter().enumerate() {
                    let _ = writeln!(out, "  {BLUE}{}. {}{RESET}", i + 1, step.as_str().unwrap_or("?"));
                }
            }
        }
        "step.start" => {
            let idx = data["index"].as_u64().unwrap_or(0);
            let total = data["total"].as_u64().unwrap_or(0);
            let desc = data["description"].as_str().unwrap_or("");
            let _ = writeln!(out, "\n{tag} {BOLD}▸ step {idx}/{total}{RESET} {desc}");
        }
        "step.end" => {
            let idx = data["index"].as_u64().unwrap_or(0);
            let result = data["result"].as_str().unwrap_or("");
            let _ = writeln!(out, "{tag} {DIM}▸ step {idx} done:{RESET} {}", truncate(result, 120));
        }
        "supervise" => {
            let action = data["action"].as_str().unwrap_or("?");
            match action {
                "complete" => { let _ = writeln!(out, "{tag} {GREEN}{BOLD}✓ complete{RESET}"); }
                "needs_work" => {
                    let feedback = data["feedback"].as_str().unwrap_or("?");
                    let _ = writeln!(out, "{tag} {YELLOW}↻ needs work:{RESET} {}", truncate(feedback, 120));
                }
                _ => { let _ = writeln!(out, "{tag} {DIM}supervise: {action}{RESET}"); }
            }
        }
        "text" => {
            // Skip in pretty — executor text is streaming noise
        }
        "done" => {
            let content = data["content"].as_str().unwrap_or("");
            let _ = writeln!(out, "\n{BOLD}━━━ Result ━━━{RESET}");
            let _ = writeln!(out, "{content}");
            let _ = writeln!(out, "{BOLD}━━━━━━━━━━━━━━{RESET}");
        }
        "context" => {
            let mem = data["memory_hits"].as_u64().unwrap_or(0);
            let sess = data["session_messages"].as_u64().unwrap_or(0);
            let _ = writeln!(out, "{DIM}context: {mem} memories, {sess} session msgs{RESET}");
        }
        "memory" => {
            let key = data["key"].as_str().unwrap_or("?");
            let action = data["action"].as_str().unwrap_or("?");
            let _ = writeln!(out, "{DIM}💾 memory {action}: {key}{RESET}");
        }
        "error" => {
            let msg = data["message"].as_str().unwrap_or("?");
            let _ = writeln!(out, "{RED}{BOLD}✗ error:{RESET} {msg}");
        }
        _ => {
            let _ = writeln!(out, "{tag} {DIM}{typ}: {}{RESET}", truncate(&data.to_string(), 100));
        }
    }
}
