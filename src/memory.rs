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
        // save_message の重複チェック/UPSERT用のWHERE句
        // (channel_id, author_id, role, content) をそのままカバーする
        // 複合インデックス。以前はchannel_id以外の列にインデックスが無く、
        // 書き込みのたびにフルスキャンに近い形になっていた。
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_messages_dedup
             ON messages(channel_id, author_id, role, content)",
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
        // 他スレッドがpanicしてもMutexをpoisonedのまま死なせず、
        // 内部値を回収して継続する
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());

        let now = chrono_now();
        let blob = f32_to_bytes(embedding);

        // 完全一致するメッセージがあれば、行を増やさず created_at /
        // embedding だけを更新する(UPSERT)。同じ発言を後日繰り返した
        // という事実自体は失わず、履歴の正確性を保つ。
        // idx_messages_dedup がこのWHERE句をそのままカバーする。
        let updated = conn.execute(
            "UPDATE messages
             SET created_at = ?1, embedding = ?2
             WHERE channel_id = ?3
               AND author_id = ?4
               AND role = ?5
               AND content = ?6",
            rusqlite::params![now, blob, channel_id, author_id, role, content],
        )?;

        if updated == 0 {
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
        }

        Ok(())
    }

    pub fn get_recent_history(
        &self,
        channel_id: &str,
        limit: i64,
    ) -> anyhow::Result<Vec<(String, String)>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
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

    /// 検索候補を取得する。
    /// `role` を指定すると、そのroleのレコードだけが検索対象になる
    /// (例: "fact" だけを検索し、user/assistantの過去発言を除外する)。
    /// これを絞らないと、AIが過去に発言した誤った回答(role="assistant")
    /// までもが次回以降の検索候補に混ざり、admin側でfactを更新しても
    /// 古い回答が検索結果に残り続けてしまう。
    /// `window` が `Some(n)` なら直近n件、`None` ならチャンネルの全メッセージが
    /// 対象になる。
    pub fn get_candidates_for_search(
        &self,
        channel_id: &str,
        role: &str,
        window: Option<i64>,
    ) -> anyhow::Result<Vec<MemoryCandidate>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());

        let candidates = match window {
            Some(limit) => {
                let mut stmt = conn.prepare(
                    "SELECT content, embedding, created_at
                     FROM messages
                     WHERE channel_id = ?1
                       AND role = ?2
                       AND embedding IS NOT NULL
                     ORDER BY created_at DESC
                     LIMIT ?3",
                )?;
                let rows = stmt.query_map(
                    rusqlite::params![channel_id, role, limit],
                    map_candidate_row,
                )?;
                rows.filter_map(|r| r.ok()).collect()
            }
            None => {
                let mut stmt = conn.prepare(
                    "SELECT content, embedding, created_at
                     FROM messages
                     WHERE channel_id = ?1
                       AND role = ?2
                       AND embedding IS NOT NULL
                     ORDER BY created_at DESC",
                )?;
                let rows = stmt.query_map(
                    rusqlite::params![channel_id, role],
                    map_candidate_row,
                )?;
                rows.filter_map(|r| r.ok()).collect()
            }
        };

        Ok(candidates)
    }

    /// role が一致する全レコードを取得する(事実管理用)。created_at昇順(古い順)。
    pub fn list_by_role(
        &self,
        channel_id: &str,
        role: &str,
    ) -> anyhow::Result<Vec<(i64, String, i64)>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT id, content, created_at FROM messages
         WHERE channel_id = ?1 AND role = ?2
         ORDER BY created_at ASC",
        )?;

        let rows = stmt.query_map(rusqlite::params![channel_id, role], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, i64>(2)?))
        })?;

        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// idを指定してレコードを削除する。削除できた場合はtrue。
    pub fn delete_by_id(&self, id: i64) -> anyhow::Result<bool> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let affected = conn.execute("DELETE FROM messages WHERE id = ?1", rusqlite::params![id])?;
        Ok(affected > 0)
    }

    /// idを指定して本文とembeddingを更新する。更新できた場合はtrue。
    pub fn update_content_by_id(
        &self,
        id: i64,
        content: &str,
        embedding: &[f32],
    ) -> anyhow::Result<bool> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let blob = f32_to_bytes(embedding);
        let now = chrono_now();
        let affected = conn.execute(
            "UPDATE messages SET content = ?1, embedding = ?2, created_at = ?3 WHERE id = ?4",
            rusqlite::params![content, blob, now, id],
        )?;
        Ok(affected > 0)
    }
}

fn map_candidate_row(row: &rusqlite::Row) -> rusqlite::Result<MemoryCandidate> {
    let content: String = row.get(0)?;
    let blob: Vec<u8> = row.get(1)?;
    let created_at: i64 = row.get(2)?;

    Ok(MemoryCandidate {
        text: content,
        embedding: bytes_to_f32(&blob),
        created_at,
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    fn count_messages(store: &HistoryStore, channel_id: &str) -> i64 {
        let conn = store.conn.lock().unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM messages WHERE channel_id = ?1",
            rusqlite::params![channel_id],
            |row| row.get(0),
        )
            .unwrap()
    }

    #[test]
    fn save_message_same_content_upserts_instead_of_duplicating() {
        let store = HistoryStore::new(":memory:").unwrap();

        store
            .save_message("ch1", "user1", "user", "こんにちは", &[0.1, 0.2])
            .unwrap();
        store
            .save_message("ch1", "user1", "user", "こんにちは", &[0.1, 0.2])
            .unwrap();

        // 完全一致するメッセージを2回保存しても行は1件のまま
        assert_eq!(count_messages(&store, "ch1"), 1);
    }

    #[test]
    fn save_message_different_content_inserts_new_row() {
        let store = HistoryStore::new(":memory:").unwrap();

        store
            .save_message("ch1", "user1", "user", "こんにちは", &[0.1, 0.2])
            .unwrap();
        store
            .save_message("ch1", "user1", "user", "さようなら", &[0.3, 0.4])
            .unwrap();

        assert_eq!(count_messages(&store, "ch1"), 2);
    }

    #[test]
    fn get_candidates_for_search_none_returns_all_messages() {
        let store = HistoryStore::new(":memory:").unwrap();

        for i in 0..5 {
            store
                .save_message(
                    "ch1",
                    "user1",
                    "user",
                    &format!("message {i}"),
                    &[i as f32],
                )
                .unwrap();
        }

        let candidates = store.get_candidates_for_search("ch1", "user", None).unwrap();
        assert_eq!(candidates.len(), 5);
    }

    #[test]
    fn get_candidates_for_search_some_limits_results() {
        let store = HistoryStore::new(":memory:").unwrap();

        for i in 0..5 {
            store
                .save_message(
                    "ch1",
                    "user1",
                    "user",
                    &format!("message {i}"),
                    &[i as f32],
                )
                .unwrap();
        }

        let candidates = store.get_candidates_for_search("ch1", "user", Some(2)).unwrap();
        assert_eq!(candidates.len(), 2);
    }
}