use std::io::Write;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

// Modes: 0=pretty (default), 1=verbose, 2=quiet, 3=debug
const MODE_PRETTY: u8 = 0;
const MODE_VERBOSE: u8 = 1;
const MODE_QUIET: u8 = 2;
const MODE_DEBUG: u8 = 3;

static MODE: AtomicU8 = AtomicU8::new(MODE_PRETTY);
static FULL_OUTPUT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static TOTAL_PROMPT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static TOTAL_COMPLETION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn set_verbose() { MODE.store(MODE_VERBOSE, Ordering::Relaxed); }
pub fn set_quiet() { MODE.store(MODE_QUIET, Ordering::Relaxed); }
pub fn set_debug() { MODE.store(MODE_DEBUG, Ordering::Relaxed); }
pub fn set_full() { FULL_OUTPUT.store(true, Ordering::Relaxed); }

fn is_full() -> bool { FULL_OUTPUT.load(Ordering::Relaxed) }

fn mode() -> u8 { MODE.load(Ordering::Relaxed) }

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

fn emit_raw(event: &serde_json::Value) {
    match mode() {
        MODE_PRETTY => pretty_print(event),
        MODE_DEBUG => debug_print(event),
        MODE_VERBOSE => {
            let mut stdout = std::io::stdout().lock();
            let _ = serde_json::to_writer(&mut stdout, event);
            let _ = stdout.write_all(b"\n");
            let _ = stdout.flush();
        }
        _ => {} // quiet
    }
}

pub fn emit(typ: &str, actor: &str, color: &str, data: serde_json::Value) {
    if mode() == MODE_QUIET { return; }
    emit_raw(&serde_json::json!({
        "type": typ,
        "ts": now(),
        "actor": actor,
        "color": color,
        "data": data,
    }));
}

// ── Colors ──────────────────────────────────────────────────────────────

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
const BROWN: &str = "\x1b[38;5;137m";

fn resolve_color(name: &str) -> &'static str {
    match name {
        "red" => RED, "green" => GREEN, "yellow" => YELLOW,
        "blue" => BLUE, "magenta" => MAGENTA, "cyan" => CYAN,
        "white" => WHITE, _ => WHITE,
    }
}

fn format_tokens(n: u64) -> String {
    if n == 0 { return String::new(); }
    if n >= 1000 { format!("{:.1}k tokens", n as f64 / 1000.0) }
    else { format!("{} tokens", n) }
}

fn format_cost(prompt: u64, completion: u64) -> String {
    // Default pricing: Gemini 3 Flash ($0.10/1M in, $0.40/1M out)
    let cost = (prompt as f64 * 0.10 + completion as f64 * 0.40) / 1_000_000.0;
    format!(", ${cost}")
}

// ── Pretty print (default) — minimal ───────────────────────────────────

fn pretty_print(event: &serde_json::Value) {
    let mut out = std::io::stderr().lock();
    let typ = event["type"].as_str().unwrap_or("?");
    let data = &event["data"];

    match typ {
        "tool.call" => {
            let name = data["name"].as_str().unwrap_or("?");
            let detail = data["args"].as_object()
                .and_then(|o| o.values().next())
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let _ = writeln!(out, "{YELLOW}💥 {name}{RESET} {detail}");
        }
        "tool.result" => {
            let result = data["result"].as_str().unwrap_or("");
            let lines: Vec<&str> = result.lines().collect();
            if is_full() || lines.len() <= 5 {
                for line in &lines {
                    let _ = writeln!(out, "{DIM}{line}{RESET}");
                }
            } else {
                for line in lines.iter().take(3) {
                    let _ = writeln!(out, "{DIM}{line}{RESET}");
                }
                let _ = writeln!(out, "{DIM}... ({} more lines){RESET}", lines.len() - 3);
            }
        }
        "turn.complete" => {
            let turn = data["turn"].as_u64().unwrap_or(0);
            let msgs = data["messages"].as_u64().unwrap_or(0);
            let tokens = data["total_tokens"].as_u64().unwrap_or(0);
            let prompt = data["prompt_tokens"].as_u64().unwrap_or(0);
            let completion = data["completion_tokens"].as_u64().unwrap_or(0);

            TOTAL_PROMPT.fetch_add(prompt, Ordering::Relaxed);
            TOTAL_COMPLETION.fetch_add(completion, Ordering::Relaxed);
            let cum_prompt = TOTAL_PROMPT.load(Ordering::Relaxed);
            let cum_completion = TOTAL_COMPLETION.load(Ordering::Relaxed);
            let cum_total = cum_prompt + cum_completion;

            let turn_str = if tokens > 0 { format_tokens(tokens) } else { format!("{msgs} msgs") };
            let total_str = format!(" (total: {}{})", format_tokens(cum_total), format_cost(cum_prompt, cum_completion));

            let _ = writeln!(out, "{BROWN}← turn {}, {}{}{}{RESET}",
                turn + 1, turn_str, format_cost(prompt, completion), total_str,
            );
        }
        "llm.error" => {
            let err = data["error"].as_str().unwrap_or("?");
            let _ = writeln!(out, "{RED}{BOLD}✗{RESET} {err}");
        }
        "error" => {
            let msg = data["message"].as_str().unwrap_or("?");
            let _ = writeln!(out, "{RED}{BOLD}✗{RESET} {msg}");
        }
        _ => {}
    }
}

// ── Debug print — detailed (old -p behavior) ───────────────────────────

fn debug_print(event: &serde_json::Value) {
    let mut out = std::io::stderr().lock();
    let typ = event["type"].as_str().unwrap_or("?");
    let actor = event["actor"].as_str().unwrap_or("");
    let data = &event["data"];
    let color = resolve_color(event["color"].as_str().unwrap_or("white"));

    let tag = if actor.is_empty() {
        format!("{DIM}[system]{RESET}")
    } else {
        format!("{color}{BOLD}{actor}{RESET}")
    };

    match typ {
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
                let _ = writeln!(out, "{tag} {DIM}→ llm turn={turn}{RESET} {content}");
            }
        }
        "llm.error" => {
            let err = data["error"].as_str().unwrap_or("?");
            let _ = writeln!(out, "{tag} {RED}✗ llm error:{RESET} {err}");
        }
        "tool.call" => {
            let name = data["name"].as_str().unwrap_or("?");
            let detail = data["args"].as_object()
                .and_then(|o| o.values().next())
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let _ = writeln!(out, "{tag} {YELLOW}💥 {name}{RESET} {detail}");
        }
        "tool.result" => {
            let name = data["name"].as_str().unwrap_or("?");
            let result = data["result"].as_str().unwrap_or("");
            let _ = writeln!(out, "{tag} {DIM}↳ {name}:{RESET}");
            for line in result.lines() {
                let _ = writeln!(out, "{DIM}  {line}{RESET}");
            }
        }
        "turn.complete" => {
            let turn = data["turn"].as_u64().unwrap_or(0);
            let msgs = data["messages"].as_u64().unwrap_or(0);
            let tokens = data["tokens"].as_u64().unwrap_or(0);
            let _ = writeln!(out, "{tag} {DIM}← turn {turn}, {msgs} msgs{}{RESET}", format_tokens(tokens));
        }
        "done" => {
            let tokens = data["total_tokens"].as_u64().unwrap_or(0);
            let _ = writeln!(out, "{tag} {GREEN}{BOLD}✓ done{}{RESET}", format_tokens(tokens));
        }
        "error" => {
            let msg = data["message"].as_str().unwrap_or("?");
            let _ = writeln!(out, "{RED}{BOLD}✗ error:{RESET} {msg}");
        }
        "text" | "tool.status" => {}
        _ => {
            let _ = writeln!(out, "{tag} {DIM}{typ}{RESET}");
        }
    }
}
