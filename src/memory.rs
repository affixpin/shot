use crate::emit;
use rusqlite::{ffi::sqlite3_auto_extension, Connection};
use serde::Deserialize;
use sqlite_vec::sqlite3_vec_init;
use std::sync::Mutex;
use zerocopy::IntoBytes;

const EMBED_DIM: usize = 3072; // gemini-embedding-001 output size

const CONSOLIDATION_PROMPT: &str = r#"Extract facts worth remembering from this conversation exchange.
Return a JSON array of objects with "key" and "content" fields.
- "key": short snake_case identifier (e.g. "user_lang", "project_name")
- "content": the fact to remember

Only extract durable facts, preferences, or decisions — not transient details.
Return [] if nothing is worth remembering.
Example: [{"key": "preferred_style", "content": "User prefers minimalist design"}]
Return ONLY the JSON array, no markdown, no explanation."#;

pub struct Memory {
    db: Mutex<Connection>,
    embed_url: String,
    embed_api_key: String,
}

#[derive(Debug)]
pub struct MemoryEntry {
    pub key: String,
    pub content: String,
    pub score: f64,
}

#[derive(Deserialize)]
struct EmbedResponse {
    embedding: EmbedValues,
}

#[derive(Deserialize)]
struct EmbedValues {
    values: Vec<f32>,
}

#[allow(clippy::missing_transmute_annotations)]
fn init_vec_extension() {
    unsafe {
        sqlite3_auto_extension(Some(std::mem::transmute(
            sqlite3_vec_init as *const (),
        )));
    }
}

fn init_db(db: &Connection) -> Result<(), Box<dyn std::error::Error>> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS memories (
            key TEXT PRIMARY KEY,
            content TEXT NOT NULL,
            created_at TEXT DEFAULT (datetime('now'))
        )"
    )?;
    db.execute_batch(&format!(
        "CREATE VIRTUAL TABLE IF NOT EXISTS memories_vec USING vec0(
            key TEXT PRIMARY KEY,
            embedding float[{EMBED_DIM}]
        )"
    ))?;
    Ok(())
}

impl Memory {
    pub fn open(db_path: &str, embed_api_key: &str, embed_url: &str) -> Result<Self, Box<dyn std::error::Error>> {
        init_vec_extension();
        let db = Connection::open(db_path)?;
        init_db(&db)?;
        Ok(Self {
            db: Mutex::new(db),
            embed_url: embed_url.to_string(),
            embed_api_key: embed_api_key.to_string(),
        })
    }

    async fn embed(&self, text: &str) -> Result<Vec<f32>, Box<dyn std::error::Error>> {
        let client = reqwest::Client::new();
        let resp = client
            .post(&self.embed_url)
            .query(&[("key", &self.embed_api_key)])
            .json(&serde_json::json!({
                "content": {"parts": [{"text": text}]}
            }))
            .send().await?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Embedding API error: {body}").into());
        }

        let data: EmbedResponse = resp.json().await?;
        Ok(data.embedding.values)
    }

    pub async fn store(&self, key: &str, content: &str) -> Result<(), Box<dyn std::error::Error>> {
        let embedding = self.embed(content).await?;
        let db = self.db.lock().unwrap();
        db.execute(
            "INSERT INTO memories (key, content) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET content = ?2, created_at = datetime('now')",
            rusqlite::params![key, content],
        )?;
        db.execute("DELETE FROM memories_vec WHERE key = ?1", [key])?;
        db.execute(
            "INSERT INTO memories_vec (key, embedding) VALUES (?1, ?2)",
            rusqlite::params![key, embedding.as_bytes()],
        )?;
        Ok(())
    }

    pub async fn recall(&self, query: &str, limit: usize) -> Result<Vec<MemoryEntry>, Box<dyn std::error::Error>> {
        let query_vec = self.embed(query).await?;
        let db = self.db.lock().unwrap();
        let mut stmt = db.prepare(
            "SELECT v.key, m.content, v.distance
             FROM memories_vec v
             JOIN memories m ON m.key = v.key
             WHERE v.embedding MATCH ?1 AND k = ?2
             ORDER BY v.distance"
        )?;
        let entries = stmt.query_map(
            rusqlite::params![query_vec.as_bytes(), limit as i64],
            |row| {
                let key: String = row.get(0)?;
                let content: String = row.get(1)?;
                let distance: f64 = row.get(2)?;
                let score = 1.0 / (1.0 + distance);
                Ok(MemoryEntry { key, content, score })
            },
        )?
        .filter_map(|r| r.ok())
        .filter(|e| e.score > 0.3)
        .collect();
        Ok(entries)
    }

    pub fn forget(&self, key: &str) -> Result<bool, Box<dyn std::error::Error>> {
        let db = self.db.lock().unwrap();
        let d1 = db.execute("DELETE FROM memories WHERE key = ?1", [key])?;
        let _ = db.execute("DELETE FROM memories_vec WHERE key = ?1", [key]);
        Ok(d1 > 0)
    }

    /// Extract and store durable facts from a conversation exchange.
    pub async fn consolidate(&self, api_base: &str, api_key: &str, user_msg: &str) {
        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{api_base}/chat/completions"))
            .header("Authorization", format!("Bearer {api_key}"))
            .json(&serde_json::json!({
                "model": "gemini-2.5-flash-lite",
                "temperature": 0.1,
                "messages": [
                    {"role": "system", "content": CONSOLIDATION_PROMPT},
                    {"role": "user", "content": user_msg}
                ]
            }))
            .send().await;

        let Ok(resp) = resp else { return };
        if !resp.status().is_success() { return; }

        #[derive(Deserialize)]
        struct Resp { choices: Vec<Choice> }
        #[derive(Deserialize)]
        struct Choice { message: Msg }
        #[derive(Deserialize)]
        struct Msg { content: Option<String> }

        let Ok(chat) = resp.json::<Resp>().await else { return };
        let text = chat.choices.into_iter().next()
            .and_then(|c| c.message.content)
            .unwrap_or_default();

        let clean = text.trim()
            .trim_start_matches("```json").trim_start_matches("```")
            .trim_end_matches("```").trim();
        if let Ok(facts) = serde_json::from_str::<Vec<serde_json::Value>>(clean) {
            for fact in facts {
                let key = fact["key"].as_str().unwrap_or("");
                let content = fact["content"].as_str().unwrap_or("");
                if !key.is_empty() && !content.is_empty() {
                    let _ = self.store(key, content).await;
                    emit::emit_system("memory", serde_json::json!({"key": key, "action": "stored"}));
                }
            }
        }
    }

    pub async fn context_for(&self, user_msg: &str) -> String {
        match self.recall(user_msg, 5).await {
            Ok(entries) if !entries.is_empty() => {
                let mut ctx = String::from("[Memory context]\n");
                for e in &entries {
                    ctx.push_str(&format!("- {}: {}\n", e.key, e.content));
                }
                ctx
            }
            _ => String::new(),
        }
    }
}
