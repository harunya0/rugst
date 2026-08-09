pub mod embedding;
pub mod memory;
pub mod search;
mod ffi;

use std::sync::Mutex;

use embedding::{EmbeddingProvider, LocalEmbedding};
use memory::HistoryStore;
pub use search::{SearchOptions, SearchResult};
use crate::search::search_similar_with_decay;

pub struct Rugst {
    // 埋め込みモデル用のロックをDB用のロック(HistoryStore内部の
    // Mutex<Connection>)とは別に持つ。以前はRugst全体を1本のMutexで
    // FFI層から囲んでいたため、埋め込み推論(CPU負荷が高い)の間は
    // 他スレッドのDB読み書きも完全にブロックされていた。
    // ロックを分離することで、あるスレッドが埋め込み計算をしている間も
    // 別スレッドはDBアクセス側の処理を進められるようになる。
    embedding: Mutex<LocalEmbedding>,
    memory: HistoryStore,
}

impl Rugst {
    pub fn new(db_path: &str) -> anyhow::Result<Self> {
        Ok(Self {
            embedding: Mutex::new(LocalEmbedding::new()?),
            memory: HistoryStore::new(db_path)?,
        })
    }
}

impl Rugst {
    // 埋め込み・DBそれぞれが内部でロックを取るようになったため、
    // このメソッド自体は &mut self ではなく &self で足りる。
    pub fn remember(
        &self,
        channel_id: &str,
        author_id: &str,
        role: &str,
        content: &str,
    ) -> anyhow::Result<()> {
        let embedding = {
            // 他スレッドがロック中にpanicしてもpoisonedのまま死なせず、
            // 内部値を回収して継続する
            let mut model = self
                .embedding
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            model.embed(content)?
        };

        self.memory.save_message(
            channel_id,
            author_id,
            role,
            content,
            &embedding,
        )?;

        Ok(())
    }
}

impl Rugst {
    pub fn search(
        &self,
        channel_id: &str,
        query: &str,
        options: &SearchOptions,
    ) -> anyhow::Result<Vec<SearchResult>> {
        let embedding = {
            let mut model = self
                .embedding
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            model.embed(query)?
        };

        let candidates =
            self.memory.get_candidates_for_search(
                channel_id,
                options.candidate_window,
            )?;

        Ok(search_similar_with_decay(
            &candidates,
            &embedding,
            options,
        ))
    }
    /// 指定チャンネル内の、指定roleのレコードを一覧取得する(事実管理用)。
    pub fn list_by_role(&self, channel_id: &str, role: &str) -> anyhow::Result<Vec<(i64, String, i64)>> {
        self.memory.list_by_role(channel_id, role)
    }

    /// idを指定してレコードを削除する。
    pub fn delete(&self, id: i64) -> anyhow::Result<bool> {
        self.memory.delete_by_id(id)
    }

    /// idを指定して本文を更新する(embeddingも再計算する)。
    pub fn update(&self, id: i64, content: &str) -> anyhow::Result<bool> {
        let embedding = {
            let mut model = self.embedding.lock().unwrap_or_else(|e| e.into_inner());
            model.embed(content)?
        };
        self.memory.update_content_by_id(id, content, &embedding)
    }
}