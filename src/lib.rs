pub mod embedding;
pub mod memory;
pub mod search;
mod ffi;

use embedding::{EmbeddingProvider, LocalEmbedding};
use memory::HistoryStore;
pub use search::{SearchOptions, SearchResult};
use crate::search::search_similar_with_decay;

pub struct Rugst {
    embedding: LocalEmbedding,
    memory: HistoryStore,
}

impl Rugst {
    pub fn new(db_path: &str) -> anyhow::Result<Self> {
        Ok(Self {
            embedding: LocalEmbedding::new()?,
            memory: HistoryStore::new(db_path)?,
        })
    }
}

impl Rugst {
    pub fn remember(
        &mut self,
        channel_id: &str,
        author_id: &str,
        role: &str,
        content: &str,
    ) -> anyhow::Result<()> {
        let embedding = self.embedding.embed(content)?;

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
        &mut self,
        channel_id: &str,
        query: &str,
        options: &SearchOptions,
    ) -> anyhow::Result<Vec<SearchResult>> {
        let embedding = self.embedding.embed(query)?;

        // 以前はここで 1000 を固定で渡しており、意味的に関連する古い
        // メッセージが検索候補から漏れる原因になっていた。
        // options.candidate_window (デフォルト None = 全件)を使う。
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
}