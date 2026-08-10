use rusqlite::Connection;
use std::sync::Mutex;

pub struct HistoryStore {
    conn: Mutex<Connection>,
}

#[derive(Debug)]
pub struct MemoryCandidate {
    pub id: i64,
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

        // FTS5(BM25によるキーワード検索)のセットアップ。
        // トークナイザには unicode61(既定)ではなく trigram を使う。
        // unicode61 は連続するCJK文字の並びを空白区切りなしでは
        // トークンに分割できず(1つの塊として扱われるため)、日本語の
        // ような分かち書きされていない文章では実質的に完全一致しか
        // 拾えない。trigramは3文字ずつの部分文字列でインデックスするため、
        // 形態素解析なしでも日本語の部分一致検索が機能する。
        //
        // 既存DBに対して初めてこのテーブルを追加する場合(マイグレーション)、
        // CREATE VIRTUAL TABLE IF NOT EXISTS 自体はエラーにならないが、
        // それだけでは既存のmessagesの行がインデックスされない。
        // そのため新規作成時だけ 'rebuild' コマンドでバックフィルする。
        let fts_table_exists: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'messages_fts'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map(|count| count > 0)?;

        conn.execute(
            "CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
                content,
                content='messages',
                content_rowid='id',
                tokenize='trigram'
            )",
            [],
        )?;

        if !fts_table_exists {
            // 外部コンテンツテーブル(messages)を読み直してFTS5インデックスを
            // 一から作り直す特殊コマンド。既存データのバックフィル用。
            conn.execute(
                "INSERT INTO messages_fts(messages_fts) VALUES('rebuild')",
                [],
            )?;
        }

        // messagesへの書き込みのたびにFTS5インデックスを追従させるトリガー。
        // save_message(INSERT/UPDATE)・update_content_by_id・delete_by_id の
        // すべての書き込み経路を個別に同期する必要がないよう、DB側で保証する。
        conn.execute(
            "CREATE TRIGGER IF NOT EXISTS messages_fts_ai AFTER INSERT ON messages BEGIN
                INSERT INTO messages_fts(rowid, content) VALUES (new.id, new.content);
             END",
            [],
        )?;
        conn.execute(
            "CREATE TRIGGER IF NOT EXISTS messages_fts_ad AFTER DELETE ON messages BEGIN
                INSERT INTO messages_fts(messages_fts, rowid, content) VALUES('delete', old.id, old.content);
             END",
            [],
        )?;
        conn.execute(
            "CREATE TRIGGER IF NOT EXISTS messages_fts_au AFTER UPDATE ON messages BEGIN
                INSERT INTO messages_fts(messages_fts, rowid, content) VALUES('delete', old.id, old.content);
                INSERT INTO messages_fts(rowid, content) VALUES (new.id, new.content);
             END",
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

        // UPSERT(重複排除)は role="fact" のレコードにのみ適用する。
        // fact は「同じ事実を後日また登録した」場合に行を増やさず
        // created_at/embeddingだけ更新するのが望ましい一方、
        // role="user"/"assistant" の会話ログにこれを適用すると、
        // ユーザーが「はい」「ありがとう」のような短い発言を別の
        // タイミングで繰り返しただけで既存行が上書きされてしまい、
        // 会話が実際には複数回行われた事実が履歴から消えてしまう
        // (get_recent_history の時系列順が壊れる)。
        // そのため会話ログは常にINSERTし、factのみ従来通りUPSERTする。
        let updated = if role == "fact" {
            // 完全一致するメッセージがあれば、行を増やさず created_at /
            // embedding だけを更新する(UPSERT)。
            // idx_messages_dedup がこのWHERE句をそのままカバーする。
            conn.execute(
                "UPDATE messages
                 SET created_at = ?1, embedding = ?2
                 WHERE channel_id = ?3
                   AND author_id = ?4
                   AND role = ?5
                   AND content = ?6",
                rusqlite::params![now, blob, channel_id, author_id, role, content],
            )?
        } else {
            0
        };

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
                    "SELECT id, content, embedding, created_at
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
                    "SELECT id, content, embedding, created_at
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

    /// FTS5(BM25)によるキーワード検索候補を取得する。bm25スコアが良い順
    /// (一致度が高い順)に並んで返る。
    ///
    /// `query` はエスケープしてFTS5のクエリ構文(AND/OR/NOT/"/カラムフィルタの
    /// ":"など)として解釈されないようにする。全体を1つのフレーズとして
    /// クオートすることで、ユーザーの入力をそのまま安全に渡せる
    /// (trigramトークナイザ下では、フレーズクオートした文字列は
    /// 「その並びの部分文字列を含む行」にマッチする)。
    ///
    /// `window` の意味は get_candidates_for_search と合わせてあり、
    /// `Some(n)` なら候補をBM25上位n件に絞り、`None` なら上限なし
    /// (実務上は極端に大きなDBだと重くなるため、既定値での運用を推奨)。
    pub fn get_fts_candidates(
        &self,
        channel_id: &str,
        role: &str,
        query: &str,
        window: Option<i64>,
    ) -> anyhow::Result<Vec<MemoryCandidate>> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return Ok(Vec::new());
        }

        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());

        // trigramトークナイザは3文字未満のクエリからは1つもトライグラムを
        // 生成できず、MATCHは常に0件を返す(trigramトークナイザの既知の
        // 制約であり、インデックス側の問題ではない)。日本語では「宇宙」
        // 「音楽」のような2文字の単語が非常に多いため、3文字未満の場合は
        // FTS5を経由せず LIKE による部分一致検索にフォールバックする。
        if trimmed.chars().count() < 3 {
            return self.get_like_candidates(channel_id, role, trimmed, window, &conn);
        }

        let fts_query = escape_fts_query(trimmed);

        // messages_fts は content='messages' の外部コンテンツテーブルなので、
        // rowid(=messages.id)経由でmessagesとJOINして channel_id/role を絞り込む。
        let candidates = match window {
            Some(limit) => {
                let mut stmt = conn.prepare(
                    "SELECT messages.id, messages.content, messages.created_at
                     FROM messages_fts
                     JOIN messages ON messages.id = messages_fts.rowid
                     WHERE messages_fts MATCH ?1
                       AND messages.channel_id = ?2
                       AND messages.role = ?3
                     ORDER BY bm25(messages_fts)
                     LIMIT ?4",
                )?;
                let rows = stmt.query_map(
                    rusqlite::params![fts_query, channel_id, role, limit],
                    map_fts_candidate_row,
                )?;
                rows.filter_map(|r| r.ok()).collect()
            }
            None => {
                let mut stmt = conn.prepare(
                    "SELECT messages.id, messages.content, messages.created_at
                     FROM messages_fts
                     JOIN messages ON messages.id = messages_fts.rowid
                     WHERE messages_fts MATCH ?1
                       AND messages.channel_id = ?2
                       AND messages.role = ?3
                     ORDER BY bm25(messages_fts)",
                )?;
                let rows = stmt.query_map(
                    rusqlite::params![fts_query, channel_id, role],
                    map_fts_candidate_row,
                )?;
                rows.filter_map(|r| r.ok()).collect()
            }
        };

        Ok(candidates)
    }

    /// get_fts_candidates の3文字未満クエリ用フォールバック。
    /// trigramトークナイザではMATCHが使えないため、messages テーブルに
    /// 直接 LIKE をかけて部分一致させる。BM25のような一致度スコアは
    /// 存在しないため、created_at降順(新しい順)で返す。
    fn get_like_candidates(
        &self,
        channel_id: &str,
        role: &str,
        query: &str,
        window: Option<i64>,
        conn: &Connection,
    ) -> anyhow::Result<Vec<MemoryCandidate>> {
        let pattern = format!("%{}%", escape_like(query));

        let candidates = match window {
            Some(limit) => {
                let mut stmt = conn.prepare(
                    "SELECT id, content, created_at
                     FROM messages
                     WHERE channel_id = ?1
                       AND role = ?2
                       AND content LIKE ?3 ESCAPE '\\'
                     ORDER BY created_at DESC
                     LIMIT ?4",
                )?;
                let rows = stmt.query_map(
                    rusqlite::params![channel_id, role, pattern, limit],
                    map_fts_candidate_row,
                )?;
                rows.filter_map(|r| r.ok()).collect()
            }
            None => {
                let mut stmt = conn.prepare(
                    "SELECT id, content, created_at
                     FROM messages
                     WHERE channel_id = ?1
                       AND role = ?2
                       AND content LIKE ?3 ESCAPE '\\'
                     ORDER BY created_at DESC",
                )?;
                let rows = stmt.query_map(
                    rusqlite::params![channel_id, role, pattern],
                    map_fts_candidate_row,
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
    let id: i64 = row.get(0)?;
    let content: String = row.get(1)?;
    let blob: Vec<u8> = row.get(2)?;
    let created_at: i64 = row.get(3)?;

    Ok(MemoryCandidate {
        id,
        text: content,
        embedding: bytes_to_f32(&blob),
        created_at,
    })
}

fn map_fts_candidate_row(row: &rusqlite::Row) -> rusqlite::Result<MemoryCandidate> {
    let id: i64 = row.get(0)?;
    let content: String = row.get(1)?;
    let created_at: i64 = row.get(2)?;

    Ok(MemoryCandidate {
        id,
        text: content,
        // FTS5候補はBM25の順位だけを使うため、embeddingは持たせない
        // (cosine_similarityは長さ0のベクトルに対して0.0を返すので、
        // 誤ってベクトル側の計算に混入しても安全側に倒れる)。
        embedding: Vec::new(),
        created_at,
    })
}

/// ユーザー入力をFTS5のMATCH式として安全に使えるようにエスケープする。
/// 全体を1つのダブルクオート文字列(フレーズ)として扱うことで、
/// AND/OR/NOT/カラムフィルタ(:)などのFTS5クエリ構文として
/// 解釈されるのを防ぐ。フレーズ内の `"` は `""` にエスケープする
/// (SQL文字列リテラルと同じ規則)。
fn escape_fts_query(query: &str) -> String {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    format!("\"{}\"", trimmed.replace('"', "\"\""))
}

/// LIKEパターンに埋め込むユーザー入力をエスケープする。
/// LIKEの特殊文字である `%` `_` と、エスケープ文字自身の `\` を
/// `\`でエスケープする(呼び出し側は `ESCAPE '\'` を付けること)。
fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
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
    fn save_message_same_fact_upserts_instead_of_duplicating() {
        let store = HistoryStore::new(":memory:").unwrap();

        store
            .save_message("ch1", "admin", "fact", "こんにちは", &[0.1, 0.2])
            .unwrap();
        store
            .save_message("ch1", "admin", "fact", "こんにちは", &[0.1, 0.2])
            .unwrap();

        // role="fact" は完全一致するメッセージを2回保存しても行は1件のまま
        assert_eq!(count_messages(&store, "ch1"), 1);
    }

    #[test]
    fn save_message_same_chat_content_always_inserts_new_row() {
        let store = HistoryStore::new(":memory:").unwrap();

        // role="user"/"assistant" の会話ログは、同じ発言(例:「はい」)を
        // 別のタイミングで繰り返しても行が上書きされず、
        // 発言のたびに新しい行として残らなければならない。
        store
            .save_message("ch1", "user1", "user", "はい", &[0.1, 0.2])
            .unwrap();
        store
            .save_message("ch1", "user1", "user", "はい", &[0.1, 0.2])
            .unwrap();

        assert_eq!(count_messages(&store, "ch1"), 2);
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

    #[test]
    fn get_fts_candidates_finds_exact_substring_match() {
        let store = HistoryStore::new(":memory:").unwrap();

        store
            .save_message("ch1", "admin", "fact", "文化祭の開催時間は9時から17時です", &[0.1])
            .unwrap();
        store
            .save_message("ch1", "admin", "fact", "焼きそば屋台は体育館の裏にあります", &[0.2])
            .unwrap();

        // trigramトークナイザなので、単語単位ではなく部分文字列でヒットする
        let results = store.get_fts_candidates("ch1", "fact", "開催時間", None).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].text.contains("開催時間"));
    }

    #[test]
    fn get_fts_candidates_filters_by_role() {
        let store = HistoryStore::new(":memory:").unwrap();

        store
            .save_message("ch1", "admin", "fact", "文化祭のテーマは宇宙です", &[0.1])
            .unwrap();
        store
            .save_message("ch1", "user1", "user", "文化祭のテーマは宇宙ですか?", &[0.2])
            .unwrap();

        // role="fact" だけを対象にした場合、user発言はヒットしない
        let results = store.get_fts_candidates("ch1", "fact", "宇宙", None).unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn get_fts_candidates_no_match_returns_empty() {
        let store = HistoryStore::new(":memory:").unwrap();

        store
            .save_message("ch1", "admin", "fact", "文化祭の開催時間は9時から17時です", &[0.1])
            .unwrap();

        let results = store.get_fts_candidates("ch1", "fact", "存在しないキーワード", None).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn get_fts_candidates_syncs_after_update_and_delete() {
        let store = HistoryStore::new(":memory:").unwrap();

        store
            .save_message("ch1", "admin", "fact", "更新前の内容です", &[0.1])
            .unwrap();
        let facts = store.list_by_role("ch1", "fact").unwrap();
        let id = facts[0].0;

        // 更新後は新しい内容でヒットし、古い内容ではヒットしなくなる
        // (トリガーがFTS5インデックスを追従させていることの確認)
        store.update_content_by_id(id, "更新後の内容です", &[0.2]).unwrap();
        assert!(store.get_fts_candidates("ch1", "fact", "更新後", None).unwrap().len() == 1);
        assert!(store.get_fts_candidates("ch1", "fact", "更新前", None).unwrap().is_empty());

        // 削除後はヒットしなくなる
        store.delete_by_id(id).unwrap();
        assert!(store.get_fts_candidates("ch1", "fact", "更新後", None).unwrap().is_empty());
    }
}