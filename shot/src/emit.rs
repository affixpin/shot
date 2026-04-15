use std::io::Write;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

// Modes: 0=pretty (default), 1=json, 2=quiet
const MODE_PRETTY: u8 = 0;
const MODE_JSON: u8 = 1;
const MODE_QUIET: u8 = 2;

static MODE: AtomicU8 = AtomicU8::new(MODE_PRETTY);
static TOTAL_PROMPT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static TOTAL_TOKENS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn set_json() { MODE.store(MODE_JSON, Ordering::Relaxed); }
pub fn set_quiet() { MODE.store(MODE_QUIET, Ordering::Relaxed); }

fn mode() -> u8 { MODE.load(Ordering::Relaxed) }

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

fn emit_raw(event: &serde_json::Value) {
    match mode() {
        MODE_PRETTY => pretty_print(event),
        MODE_JSON => {
            let mut stdout = std::io::stdout().lock();
            let _ = serde_json::to_writer(&mut stdout, event);
            let _ = stdout.write_all(b"\n");
            let _ = stdout.flush();
        }
        _ => {} // quiet
    }
}

pub fn emit(typ: &str, data: serde_json::Value) {
    if mode() == MODE_QUIET { return; }
    emit_raw(&serde_json::json!({
        "type": typ,
        "ts": now(),
        "data": data,
    }));
}

// ── Colors ──────────────────────────────────────────────────────────────

const RESET: &str = "\x1b[0m";
const DIM: &str = "\x1b[2m";
const BOLD: &str = "\x1b[1m";
const RED: &str = "\x1b[31m";
const YELLOW: &str = "\x1b[33m";
const BROWN: &str = "\x1b[38;5;137m";

fn format_tokens(n: u64) -> String {
    if n == 0 { return String::new(); }
    if n >= 1000 { format!("{:.1}k tokens", n as f64 / 1000.0) }
    else { format!("{} tokens", n) }
}

fn format_cost(prompt: u64, total: u64) -> String {
    // Default pricing: Gemini 3 Flash ($0.10/1M in, $0.40/1M out)
    // Output = total - prompt (includes thinking tokens which Gemini bills as output)
    let output = total.saturating_sub(prompt);
    let cost = (prompt as f64 * 0.10 + output as f64 * 0.40) / 1_000_000.0;
    format!(", ${cost}")
}

// ── Pretty print (default) — minimal ───────────────────────────────────

fn pretty_print(event: &serde_json::Value) {
    let mut out = std::io::stderr().lock();
    let typ = event["type"].as_str().unwrap_or("?");
    let data = &event["data"];

    match typ {
        "llm.response" => {
            let content = data["content"].as_str().unwrap_or("");
            let tc = data["tool_calls"].as_u64().unwrap_or(0);
            // Only show intermediate reasoning (text that precedes tool calls).
            // Final answer (tc == 0) goes to stdout via print_result.
            if !content.is_empty() && tc > 0 {
                let _ = writeln!(out, "{content}");
            }
        }
        "user.message" => {
            let content = data["content"].as_str().unwrap_or("");
            let _ = writeln!(out, "{BOLD}> {content}{RESET}");
        }
        "tool.call" => {
            let name = data["name"].as_str().unwrap_or("?");
            let detail = data["args"].as_object()
                .map(|o| {
                    o.iter()
                        .map(|(k, v)| {
                            let vs = v.as_str().map(String::from).unwrap_or_else(|| v.to_string());
                            format!("{k}={vs}")
                        })
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .unwrap_or_default();
            let _ = writeln!(out, "{YELLOW}💥 {name}{RESET} {detail}");
        }
        "tool.result" => {
            let result = data["result"].as_str().unwrap_or("");
            for line in result.lines() {
                let _ = writeln!(out, "{DIM}{line}{RESET}");
            }
        }
        "turn.complete" => {
            // Accumulate silently; final stats printed on "done"
            let tokens = data["total_tokens"].as_u64().unwrap_or(0);
            let prompt = data["prompt_tokens"].as_u64().unwrap_or(0);
            TOTAL_PROMPT.fetch_add(prompt, Ordering::Relaxed);
            TOTAL_TOKENS.fetch_add(tokens, Ordering::Relaxed);
        }
        "done" => {
            let cum_prompt = TOTAL_PROMPT.load(Ordering::Relaxed);
            let cum_total = TOTAL_TOKENS.load(Ordering::Relaxed);
            if cum_total > 0 {
                let _ = writeln!(out, "{BROWN}← {}{}{RESET}",
                    format_tokens(cum_total),
                    format_cost(cum_prompt, cum_total),
                );
            }
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

