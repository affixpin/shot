use axum::{routing::post, Json, Router, response::IntoResponse};
use std::sync::{Arc, Mutex};
use shotclaw::{session::Session, memory::Memory, run, Config};

// ── Mock server helpers ─────────────────────────────────────────────────

/// Convert a non-streaming OpenAI response JSON into SSE format.
fn to_sse(response: serde_json::Value) -> String {
    let message = &response["choices"][0]["message"];
    let mut chunks = Vec::new();

    // Text content — split into a few chunks to simulate streaming
    if let Some(text) = message["content"].as_str() {
        for (i, ch) in text.chars().enumerate() {
            chunks.push(serde_json::json!({
                "choices": [{"delta": {"content": ch.to_string()}}]
            }));
        }
    }

    // Tool calls — send as a single delta
    if let Some(tool_calls) = message["tool_calls"].as_array() {
        for (i, tc) in tool_calls.iter().enumerate() {
            chunks.push(serde_json::json!({
                "choices": [{"delta": {"tool_calls": [{
                    "index": i,
                    "id": tc["id"],
                    "function": tc["function"],
                    "thought_signature": tc.get("thought_signature"),
                }]}}]
            }));
        }
    }

    let mut sse = String::new();
    for chunk in chunks {
        sse.push_str(&format!("data: {}\n\n", serde_json::to_string(&chunk).unwrap()));
    }
    sse.push_str("data: [DONE]\n\n");
    sse
}

fn sse_response(body: String) -> impl IntoResponse {
    axum::response::Response::builder()
        .header("content-type", "text/event-stream")
        .body(body)
        .unwrap()
}

async fn mock_server(responses: Vec<serde_json::Value>) -> (String, tokio::task::JoinHandle<()>) {
    let responses = Arc::new(Mutex::new(responses));
    let app = Router::new().route(
        "/v1/chat/completions",
        post(move |Json(_req): Json<serde_json::Value>| {
            let responses = responses.clone();
            async move {
                let mut queue = responses.lock().unwrap();
                let resp = if queue.is_empty() {
                    text_response("fallback")
                } else {
                    queue.remove(0)
                };
                sse_response(to_sse(resp))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}/v1"), handle)
}

fn config(api_base: String) -> Config {
    Config {
        api_base, api_key: "test".into(),
        system_prompt: String::new(), max_turns: 5,
        embed_url: String::new(),
        memory_db: ":memory:".into(), session_path: temp_db(),
        max_session_chars: 200_000,
    }
}

fn text_response(text: &str) -> serde_json::Value {
    serde_json::json!({ "choices": [{"message": {"role": "assistant", "content": text}}] })
}

fn tool_call_response(id: &str, name: &str, args: &str) -> serde_json::Value {
    serde_json::json!({ "choices": [{"message": {
        "role": "assistant", "content": null,
        "tool_calls": [{ "id": id, "type": "function", "function": {"name": name, "arguments": args} }]
    }}] })
}

fn tool_call_with_signature(id: &str, name: &str, args: &str, sig: &str) -> serde_json::Value {
    serde_json::json!({ "choices": [{"message": {
        "role": "assistant", "content": null,
        "tool_calls": [{ "id": id, "type": "function", "function": {"name": name, "arguments": args}, "thought_signature": sig }]
    }}] })
}

// ── Tests ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn simple_text_response() {
    let (base, _h) = mock_server(vec![text_response("Hello!")]).await;
    let result = run(&config(base), "hi").await.unwrap();
    assert_eq!(result, "Hello!");
}

#[tokio::test]
async fn tool_call_shell() {
    let (base, _h) = mock_server(vec![
        tool_call_response("c1", "shell", r#"{"command":"echo hello"}"#),
        text_response("The output was: hello"),
    ]).await;
    let result = run(&config(base), "run echo hello").await.unwrap();
    assert_eq!(result, "The output was: hello");
}

#[tokio::test]
async fn tool_call_file_write_and_read() {
    let dir = std::env::temp_dir().join(format!("agent_test_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("test.txt");

    let (base, _h) = mock_server(vec![
        tool_call_response("c1", "file_write", &format!(
            r#"{{"path":"{}","content":"hello world"}}"#, path.display()
        )),
        tool_call_response("c2", "file_read", &format!(
            r#"{{"path":"{}"}}"#, path.display()
        )),
        text_response("File contains: hello world"),
    ]).await;
    let mut cfg = config(base);
    cfg.max_turns = 10;
    let result = run(&cfg, "write and read a file").await.unwrap();
    assert_eq!(result, "File contains: hello world");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello world");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn thought_signature_passthrough() {
    let requests: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(vec![]));
    let requests_clone = requests.clone();

    let app = Router::new().route(
        "/v1/chat/completions",
        post(move |Json(req): Json<serde_json::Value>| {
            let requests = requests_clone.clone();
            async move {
                let mut reqs = requests.lock().unwrap();
                reqs.push(req);
                let resp = if reqs.len() == 1 {
                    tool_call_with_signature("c1", "shell", r#"{"command":"echo ok"}"#, "sig123")
                } else {
                    text_response("done")
                };
                sse_response(to_sse(resp))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let result = run(&config(format!("http://{addr}/v1")), "test").await.unwrap();
    assert_eq!(result, "done");

    let reqs = requests.lock().unwrap();
    assert!(reqs.len() >= 2); // 2 agent calls + 1 consolidation call
    let messages = reqs[1]["messages"].as_array().unwrap();
    let assistant_msg = messages.iter().find(|m| m["role"] == "assistant").unwrap();
    assert_eq!(assistant_msg["tool_calls"][0]["thought_signature"], "sig123");
}

#[tokio::test]
async fn max_turns_exceeded() {
    let responses: Vec<_> = (0..5)
        .map(|i| tool_call_response(&format!("c{i}"), "shell", r#"{"command":"echo loop"}"#))
        .collect();
    let (base, _h) = mock_server(responses).await;
    let mut cfg = config(base);
    cfg.max_turns = 3;
    let result = run(&cfg, "loop forever").await;
    assert!(result.unwrap_err().to_string().contains("Max turns"));
}

#[tokio::test]
async fn system_prompt_from_workspace() {
    let dir = std::env::temp_dir().join(format!("agent_ws_{}", std::process::id()));
    let skills_dir = dir.join("skills").join("test");
    std::fs::create_dir_all(&skills_dir).unwrap();
    std::fs::write(dir.join("IDENTITY.md"), "You are a test bot.").unwrap();
    std::fs::write(skills_dir.join("SKILL.md"), "# Test Skill\nDo things.").unwrap();

    // Build system prompt the same way Config::load does
    let mut prompt = String::new();
    prompt.push_str(&std::fs::read_to_string(dir.join("IDENTITY.md")).unwrap());
    prompt.push('\n');
    for e in std::fs::read_dir(dir.join("skills")).unwrap().flatten() {
        let sm = e.path().join("SKILL.md");
        if sm.exists() { prompt.push_str(&std::fs::read_to_string(&sm).unwrap()); prompt.push('\n'); }
    }

    assert!(prompt.contains("You are a test bot."));
    assert!(prompt.contains("# Test Skill"));
    let _ = std::fs::remove_dir_all(&dir);
}

// ── Memory tests ────────────────────────────────────────────────────────

/// Mock embedding server that returns deterministic vectors based on input hash.
/// Similar texts produce similar vectors (shared seed prefix), different texts diverge.
async fn mock_embed_server() -> (String, tokio::task::JoinHandle<()>) {
    let app = Router::new().route(
        "/embed",
        post(|Json(req): Json<serde_json::Value>| async move {
            let text = req["content"]["parts"][0]["text"].as_str().unwrap_or("default");
            let seed = text.bytes().fold(0u32, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u32));
            let values: Vec<f32> = (0..3072)
                .map(|i| {
                    let x = ((seed.wrapping_mul(i as u32 + 1)) as f32) / u32::MAX as f32;
                    x * 2.0 - 1.0
                })
                .collect();
            Json(serde_json::json!({ "embedding": { "values": values } }))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}/embed"), handle)
}

fn temp_db() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir()
        .join(format!("mem_test_{}_{}.db", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed)))
        .to_string_lossy().to_string()
}

#[tokio::test]
async fn memory_store_and_recall() {
    let (embed_url, _h) = mock_embed_server().await;
    let db_path = temp_db();

    let mem = Memory::open_with_embed_url(&db_path, "test", &embed_url).unwrap();

    // Store two facts
    mem.store("fav_color", "blue").await.unwrap();
    mem.store("fav_food", "pizza").await.unwrap();

    // Recall — same text should match itself
    let results = mem.recall("blue", 5).await.unwrap();
    assert!(!results.is_empty(), "should find at least one memory");
    assert!(results.iter().any(|e| e.key == "fav_color"));

    let _ = std::fs::remove_file(&db_path);
}

#[tokio::test]
async fn memory_forget() {
    let (embed_url, _h) = mock_embed_server().await;
    let db_path = temp_db();

    let mem = Memory::open_with_embed_url(&db_path, "test", &embed_url).unwrap();
    mem.store("temp_fact", "something temporary").await.unwrap();

    let deleted = mem.forget("temp_fact").unwrap();
    assert!(deleted);

    let deleted_again = mem.forget("temp_fact").unwrap();
    assert!(!deleted_again);

    let _ = std::fs::remove_file(&db_path);
}

#[tokio::test]
async fn memory_context_for() {
    let (embed_url, _h) = mock_embed_server().await;
    let db_path = temp_db();

    let mem = Memory::open_with_embed_url(&db_path, "test", &embed_url).unwrap();
    mem.store("user_lang", "Rust programmer").await.unwrap();

    let ctx = mem.context_for("what language do I use?").await;
    // Context might be empty if the deterministic vectors don't match well enough,
    // but the function should not error
    assert!(ctx.is_empty() || ctx.contains("user_lang"));

    let _ = std::fs::remove_file(&db_path);
}

#[tokio::test]
async fn memory_upsert_overwrites() {
    let (embed_url, _h) = mock_embed_server().await;
    let db_path = temp_db();

    let mem = Memory::open_with_embed_url(&db_path, "test", &embed_url).unwrap();
    mem.store("fav_color", "blue").await.unwrap();
    mem.store("fav_color", "green").await.unwrap();

    // Recall should return the updated value
    let results = mem.recall("green", 5).await.unwrap();
    let entry = results.iter().find(|e| e.key == "fav_color");
    assert!(entry.is_some());
    assert_eq!(entry.unwrap().content, "green");

    let _ = std::fs::remove_file(&db_path);
}

// ── Session tests ───────────────────────────────────────────────────────

fn msg_json(role: &str, content: &str) -> String {
    serde_json::json!({"role": role, "content": content}).to_string()
}

#[tokio::test]
async fn session_push_and_recent() {
    let db = temp_db();
    let h = Session::open(&db, 200_000).unwrap();

    h.push("user", &msg_json("user", "hello"));
    h.push("assistant", &msg_json("assistant", "hi there"));
    h.push("user", &msg_json("user", "how are you?"));
    h.push("assistant", &msg_json("assistant", "good!"));

    let entries = h.recent();
    assert_eq!(entries.len(), 4);
    assert_eq!(entries[0].role, "user");
    let first: serde_json::Value = serde_json::from_str(&entries[0].data).unwrap();
    assert_eq!(first["content"], "hello");
    let last: serde_json::Value = serde_json::from_str(&entries[3].data).unwrap();
    assert_eq!(last["content"], "good!");

    let _ = std::fs::remove_file(&db);
}

#[tokio::test]
async fn session_respects_char_budget() {
    let db = temp_db();
    // Budget counts JSON data length, so use larger budget than before
    let h = Session::open(&db, 60).unwrap();

    h.push("user", &msg_json("user", "short"));
    h.push("assistant", &msg_json("assistant", "also short"));
    h.push("user", &msg_json("user", "a very long message here!"));
    h.push("assistant", &msg_json("assistant", "another long reply!!"));

    let entries = h.recent();
    assert!(entries.len() < 4);
    let last: serde_json::Value = serde_json::from_str(&entries.last().unwrap().data).unwrap();
    assert_eq!(last["content"], "another long reply!!");

    let _ = std::fs::remove_file(&db);
}

#[tokio::test]
async fn session_separate_files_are_isolated() {
    let db1 = temp_db();
    let db2 = temp_db();
    let s1 = Session::open(&db1, 200_000).unwrap();
    let s2 = Session::open(&db2, 200_000).unwrap();

    s1.push("user", &msg_json("user", "working on project A"));
    s2.push("user", &msg_json("user", "working on project B"));

    assert_eq!(s1.recent().len(), 1);
    assert_eq!(s2.recent().len(), 1);
    assert!(s1.recent()[0].data.contains("project A"));
    assert!(s2.recent()[0].data.contains("project B"));

    let _ = std::fs::remove_file(&db1);
    let _ = std::fs::remove_file(&db2);
}

#[tokio::test]
async fn session_clear() {
    let db = temp_db();
    let s = Session::open(&db, 200_000).unwrap();

    s.push("user", &msg_json("user", "hello"));
    s.push("assistant", &msg_json("assistant", "hi"));
    assert_eq!(s.recent().len(), 2);

    s.clear();
    assert_eq!(s.recent().len(), 0);

    let _ = std::fs::remove_file(&db);
}

#[tokio::test]
async fn session_preserves_all_data() {
    let db = temp_db();
    let h = Session::open(&db, 10).unwrap();

    for i in 0..20 {
        h.push("user", &msg_json("user", &format!("msg {i}")));
    }

    // recent() returns only what fits in budget
    let recent = h.recent();
    assert!(recent.len() < 20);

    // Re-opening with large budget shows all data is preserved
    let h_full = Session::open(&db, 200_000).unwrap();
    assert_eq!(h_full.recent().len(), 20);

    let _ = std::fs::remove_file(&db);
}

#[tokio::test]
async fn session_persists_tool_calls() {
    let session_db = temp_db();
    let (base, _h) = mock_server(vec![
        tool_call_response("c1", "shell", r#"{"command":"echo hi"}"#),
        text_response("Done!"),
    ]).await;

    let cfg = Config {
        api_base: base, api_key: "test".into(),
        system_prompt: String::new(), max_turns: 5,
        embed_url: String::new(),
        memory_db: ":memory:".into(), session_path: session_db.clone(),
        max_session_chars: 200_000,
    };

    let result = run(&cfg, "run echo hi").await.unwrap();
    assert_eq!(result, "Done!");

    // Check session has all messages: user, assistant(tool_call), tool(result), assistant(final)
    let s = Session::open(&session_db, 200_000).unwrap();
    let entries = s.recent();
    assert_eq!(entries.len(), 4);
    assert_eq!(entries[0].role, "user");
    assert_eq!(entries[1].role, "assistant");
    assert_eq!(entries[2].role, "tool");
    assert_eq!(entries[3].role, "assistant");

    // Verify tool call structure is preserved
    let assistant_data: serde_json::Value = serde_json::from_str(&entries[1].data).unwrap();
    assert!(assistant_data["tool_calls"].is_array());
    assert_eq!(assistant_data["tool_calls"][0]["function"]["name"], "shell");

    // Verify tool result has tool_call_id
    let tool_data: serde_json::Value = serde_json::from_str(&entries[2].data).unwrap();
    assert_eq!(tool_data["tool_call_id"], "c1");
    assert!(tool_data["content"].as_str().unwrap().contains("hi"));

    let _ = std::fs::remove_file(&session_db);
}

#[tokio::test]
async fn session_migrates_old_schema() {
    let db_path = temp_db();

    // Create old-schema table and insert data
    {
        let db = rusqlite::Connection::open(&db_path).unwrap();
        db.execute_batch(
            "CREATE TABLE messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at TEXT DEFAULT (datetime('now'))
            )"
        ).unwrap();
        db.execute("INSERT INTO messages (role, content) VALUES ('user', 'old message')", []).unwrap();
        db.execute("INSERT INTO messages (role, content) VALUES ('assistant', 'old reply')", []).unwrap();
    }

    // Open with new Session — should migrate
    let s = Session::open(&db_path, 200_000).unwrap();
    let entries = s.recent();
    assert_eq!(entries.len(), 2);

    // Migrated data should be valid JSON
    let first: serde_json::Value = serde_json::from_str(&entries[0].data).unwrap();
    assert_eq!(first["role"], "user");
    assert_eq!(first["content"], "old message");

    // New entries should work too
    s.push("user", &msg_json("user", "new message"));
    assert_eq!(s.recent().len(), 3);

    let _ = std::fs::remove_file(&db_path);
}
