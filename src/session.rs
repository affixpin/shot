use crate::react::Message;
use rusqlite::Connection;
use std::sync::Mutex;

pub struct Session {
    db: Mutex<Connection>,
    max_chars: usize,
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
        result
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
