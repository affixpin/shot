use crate::react::Message;
use rusqlite::Connection;
use std::path::Path;
use std::sync::Mutex;

pub struct Session {
    db: Mutex<Connection>,
    max_chars: usize,
}

pub struct SessionInfo {
    pub name: String,
    pub size_bytes: u64,
    pub message_count: usize,
}

impl Session {
    pub fn open(db_path: &str, max_chars: usize) -> Result<Self, Box<dyn std::error::Error>> {
        if let Some(parent) = std::path::Path::new(db_path).parent() {
            std::fs::create_dir_all(parent)?;
        }
        let db = Connection::open(db_path)?;

        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                role TEXT NOT NULL,
                data TEXT NOT NULL,
                created_at TEXT DEFAULT (datetime('now'))
            )"
        )?;

        Ok(Self { db: Mutex::new(db), max_chars })
    }

    pub fn recent(&self) -> Vec<Message> {
        let db = self.db.lock().unwrap();
        let mut stmt = db.prepare(
            "SELECT data FROM messages ORDER BY id DESC"
        ).unwrap();

        let all: Vec<(String, usize)> = stmt.query_map([], |row| {
            let data: String = row.get(0)?;
            let len = data.len();
            Ok((data, len))
        }).unwrap().filter_map(|r| r.ok()).collect();

        let mut result = Vec::new();
        let mut chars = 0;
        for (data, len) in &all {
            chars += len;
            if chars > self.max_chars {
                break;
            }
            if let Ok(msg) = serde_json::from_str::<Message>(data) {
                result.push(msg);
            }
        }
        result.reverse();

        // Truncate from the front until we hit a user message boundary.
        // This prevents replaying history that starts with an orphaned
        // assistant(tool_calls) or tool_result, which providers like Gemini reject.
        while let Some(first) = result.first() {
            if first.role == "user" { break; }
            result.remove(0);
        }

        result
    }

    /// Enumerate sessions in `dir`. Returns sessions sorted by size descending.
    /// Empty vec if the directory is missing, unreadable, or contains no sessions.
    pub fn list(dir: &Path) -> Vec<SessionInfo> {
        let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new(); };

        let mut sessions = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("db") { continue; }
            let Some(name) = path.file_stem().and_then(|s| s.to_str()).map(String::from) else { continue; };
            let size_bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
            let message_count = Connection::open(&path)
                .ok()
                .and_then(|db| db.query_row("SELECT COUNT(*) FROM messages", [], |row| row.get::<_, i64>(0)).ok())
                .unwrap_or(0) as usize;
            sessions.push(SessionInfo { name, size_bytes, message_count });
        }

        sessions.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
        sessions
    }

    pub fn push(&self, msg: &Message) {
        let role = &msg.role;
        let data = serde_json::to_string(msg).unwrap_or_default();
        let db = self.db.lock().unwrap();
        let _ = db.execute(
            "INSERT INTO messages (role, data) VALUES (?1, ?2)",
            rusqlite::params![role, data],
        );
    }
}
