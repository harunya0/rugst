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
        // 他スレッドがpanicしてもMutexをpoisonedのまま死なせず、
        // 内部値を回収して処理を継続する
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());

        let now = chrono_now();
        let blob = f32_to_bytes(embedding);

        // 以前は完全一致するメッセージがあると保存自体をスキップしていたが、
        // それだと「同じ内容の発言を後日繰り返した」という事実自体が
        // 履歴から消えてしまい、HistoryStoreの目的(会話履歴の正確な記録)と
        // 矛盾していた。now は最新の再送を、created_at / embedding だけを
        // 更新するUPSERTに変更する(発言そのものの重複行は増やさない)。
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
    /// `window` が `Some(n)` なら直近n件、`None` ならチャンネルの全メッセージが
    /// 対象になる。以前は呼び出し元(lib.rs)が常に1000固定で呼んでおり、
    /// それより古いメッセージは意味的に関連していても検索対象から
    /// 事前に除外されてしまっていた(「意味的類似度 + 時間減衰」という
    /// 設計思想に反していた)。
    pub fn get_candidates_for_search(
        &self,
        channel_id: &str,
        window: Option<i64>,
    ) -> anyhow::Result<Vec<MemoryCandidate>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());

        let candidates = match window {
            Some(limit) => {
                let mut stmt = conn.prepare(
                    "SELECT content, embedding, created_at
                     FROM messages
                     WHERE channel_id = ?1
                       AND embedding IS NOT NULL
                     ORDER BY created_at DESC
                     LIMIT ?2",
                )?;
                let rows = stmt.query_map(
                    rusqlite::params![channel_id, limit],
                    map_candidate_row,
                )?;
                rows.filter_map(|r| r.ok()).collect()
            }
            None => {
                let mut stmt = conn.prepare(
                    "SELECT content, embedding, created_at
                     FROM messages
                     WHERE channel_id = ?1
                       AND embedding IS NOT NULL
                     ORDER BY created_at DESC",
                )?;
                let rows = stmt.query_map(
                    rusqlite::params![channel_id],
                    map_candidate_row,
                )?;
                rows.filter_map(|r| r.ok()).collect()
            }
        };

        Ok(candidates)
    }

    // list_sessions / delete_session は "channel_id" が "channel:session"
    // 形式であることを前提にしていたが、remember() / save_message() を含め
    // どこにもその形式でchannel_idを組み立てる処理が存在せず、常に空を
    // 返すだけの到達不能コードだった。呼び出し元も存在しなかったため削除。
    // セッション分割が必要になった場合は、channel_idの組み立てルールを
    // 含めて設計し直す必要がある。
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