use rusqlite::Connection;
use std::sync::Mutex;

pub struct HistoryStore {
    conn: Mutex<Connection>,
}

#[derive(Debug)]
pub struct MemoryCandidate {
    pub text: String,
    pub embedding: Vec<f32>,
    pub created_at: i64,
}

impl HistoryStore {
    pub fn new(db_path: &str) -> anyhow::Result<Self> {
        let conn = Connection::open(db_path)?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                channel_id TEXT NOT NULL,
                author_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                embedding BLOB,
                created_at INTEGER NOT NULL
            )",
            [],
        )?;
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_messages_channel ON messages(channel_id, created_at)",
            [],
        )?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn save_message(
        &self,
        channel_id: &str,
        author_id: &str,
        role: &str,
        content: &str,
        embedding: &[f32],
    ) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();

        let exist: bool = conn.query_row(
            "SELECT EXISTS(
            SELECT 1
            FROM messages
            WHERE channel_id = ?1
                AND author_id = ?2
                AND role = ?3
                AND content = ?4
            )",
            rusqlite::params![channel_id, author_id, role, content],
            |row| row.get(0),
        )?;

        if exist {
            return Ok(());
        }

        let now = chrono_now();
        let blob = f32_to_bytes(embedding);

        conn.execute(
            "INSERT INTO messages
            (channel_id, author_id, role, content, embedding, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                channel_id,
                author_id,
                role,
                content,
                blob,
                now
            ],
        )?;
        Ok(())
    }

    pub fn get_recent_history(
        &self,
        channel_id: &str,
        limit: i64,
    ) -> anyhow::Result<Vec<(String, String)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT role, content FROM messages
             WHERE channel_id = ?1
             ORDER BY created_at DESC
             LIMIT ?2",
        )?;

        let rows = stmt.query_map(rusqlite::params![channel_id, limit], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;

        let mut result: Vec<(String, String)> = rows.filter_map(|r| r.ok()).collect();
        result.reverse(); // 古い順に並べ直す(AIへのプロンプトは時系列順が自然)
        Ok(result)
    }

    pub fn get_candidates_for_search(
        &self,
        channel_id: &str,
        window: i64,
    ) -> anyhow::Result<Vec<MemoryCandidate>> {
        let conn = self.conn.lock().unwrap();

        let mut stmt = conn.prepare(
            "SELECT content, embedding, created_at
         FROM messages
         WHERE channel_id = ?1
           AND embedding IS NOT NULL
         ORDER BY created_at DESC
         LIMIT ?2",
        )?;

        let rows = stmt.query_map(
            rusqlite::params![channel_id, window],
            |row| {
                let content: String = row.get(0)?;
                let blob: Vec<u8> = row.get(1)?;
                let created_at: i64 = row.get(2)?;

                Ok(MemoryCandidate {
                    text: content,
                    embedding: bytes_to_f32(&blob),
                    created_at,
                })
            },
        )?;

        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn list_sessions(&self, channel_id: &str) -> anyhow::Result<Vec<String>> {
        let conn = self.conn.lock().unwrap();
        let pattern = format!("{}:%", channel_id);
        let mut stmt = conn.prepare(
            "SELECT DISTINCT channel_id FROM messages WHERE channel_id LIKE ?1"
        )?;
        let rows = stmt.query_map(rusqlite::params![pattern], |row| {
            row.get::<_, String>(0)
        })?;
        let sessions: Vec<String> = rows
            .filter_map(|r| r.ok())
            .filter_map(|full| full.split(':').nth(1).map(|s| s.to_string()))
            .collect();
        Ok(sessions)
    }

    pub fn delete_session(&self, channel_id: &str, session: &str) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        let key = format!("{}:{}", channel_id, session);
        conn.execute("DELETE FROM messages WHERE channel_id = ?1", rusqlite::params![key])?;
        Ok(())
    }
}

fn chrono_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}
pub fn f32_to_bytes(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

pub fn bytes_to_f32(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}