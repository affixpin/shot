use std::io::Write;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const MODE_QUIET: u8 = 0;
const MODE_VERBOSE: u8 = 1;
const MODE_PRETTY: u8 = 2;

static MODE: AtomicU8 = AtomicU8::new(MODE_QUIET);

pub fn set_verbose() {
    MODE.store(MODE_VERBOSE, Ordering::Relaxed);
}

pub fn set_pretty() {
    MODE.store(MODE_PRETTY, Ordering::Relaxed);
}

fn mode() -> u8 {
    MODE.load(Ordering::Relaxed)
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

fn emit_raw(event: &serde_json::Value) {
    match mode() {
        MODE_PRETTY => pretty_print(event),
        MODE_VERBOSE => {
            let mut stdout = std::io::stdout().lock();
            let _ = serde_json::to_writer(&mut stdout, event);
            let _ = stdout.write_all(b"\n");
            let _ = stdout.flush();
        }
        _ => {}
    }
}

/// Emit an event with actor and color.
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

// ── Pretty printer ──────────────────────────────────────────────────────

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

fn resolve_color(name: &str) -> &'static str {
    match name {
        "red" => RED,
        "green" => GREEN,
        "yellow" => YELLOW,
        "blue" => BLUE,
        "magenta" => MAGENTA,
        "cyan" => CYAN,
        "white" => WHITE,
        _ => WHITE,
    }
}

fn pretty_print(event: &serde_json::Value) {
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
                let _ = writeln!(out, "{tag} {DIM}→ llm turn={turn}{RESET} {}", content);
            } else {
                let _ = writeln!(out, "{tag} {DIM}→ llm turn={turn} (empty){RESET}");
            }
        }
        "llm.error" => {
            let err = data["error"].as_str().unwrap_or("?");
            let _ = writeln!(out, "{tag} {RED}✗ llm error:{RESET} {}", err);
        }
        "tool.call" => {
            let name = data["name"].as_str().unwrap_or("?");
            let args = &data["args"];
            let detail = args.as_object()
                .and_then(|o| o.values().next())
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let _ = writeln!(out, "{tag} {YELLOW}⚡ {name}{RESET} {}", detail);
        }
        "tool.result" => {
            let name = data["name"].as_str().unwrap_or("?");
            let result = data["result"].as_str().unwrap_or("");
            let _ = writeln!(out, "{tag} {DIM}  ↳ {name}:{RESET}");
            for line in result.lines() {
                let _ = writeln!(out, "  {DIM}  {line}{RESET}");
            }
        }
        "done" => {
            let _ = writeln!(out, "{tag} {GREEN}{BOLD}✓ done{RESET}");
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
