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

        let has_table: bool = db.query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='messages'",
            [], |row| row.get::<_, i64>(0),
        ).map(|c| c > 0).unwrap_or(false);

        if !has_table {
            db.execute_batch(
                "CREATE TABLE messages (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    role TEXT NOT NULL,
                    data TEXT NOT NULL,
                    created_at TEXT DEFAULT (datetime('now'))
                )"
            )?;
        } else {
            // Migrate old schema: content → data
            let has_data: bool = db.prepare("PRAGMA table_info(messages)")?
                .query_map([], |row| row.get::<_, String>(1))?
                .filter_map(|r| r.ok())
                .any(|col| col == "data");

            if !has_data {
                db.execute_batch(
                    "CREATE TABLE messages_new (
                        id INTEGER PRIMARY KEY AUTOINCREMENT,
                        role TEXT NOT NULL,
                        data TEXT NOT NULL,
                        created_at TEXT DEFAULT (datetime('now'))
                    );
                    INSERT INTO messages_new (id, role, data, created_at)
                        SELECT id, role, json_object('role', role, 'content', content), created_at
                        FROM messages;
                    DROP TABLE messages;
                    ALTER TABLE messages_new RENAME TO messages;"
                )?;
            }
        }

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
