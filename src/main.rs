mod embedding;
mod search;
mod memory;

use anyhow::Result;
use embedding::{EmbeddingProvider, LocalEmbedding};
use std::time::Instant;
use crate::memory::HistoryStore;
use crate::search::SearchOptions;

fn main() -> Result<()> {
    let all = Instant::now();
    let start = Instant::now();
    let mut embedding = LocalEmbedding::new()?;
    println!("モデル初期化: {:?}", start.elapsed());

    let memory = HistoryStore::new("rugst.db")?;

    let text = "こんにちは、今日はRustを書いています";
    let vector = embedding.embed(text)?;

    memory.save_message(
        "test",
        "user",
        "user",
        text,
        &vector,
    )?;

    let candidates = memory.get_candidates_for_search("test", 100)?;
    let query = embedding.embed("Rustについて話した記憶")?;
    let options = SearchOptions::default();

    let results = search::search_similar_with_decay(
        &candidates,
        &query,
        &options,
    );
    println!("検索結果: {:?}", results);
    println!("全体: {:?}", all.elapsed());

    Ok(())
}