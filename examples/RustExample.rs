use anyhow::Result;
use rugst::{Rugst, SearchOptions};

fn main() -> Result<()> {
    let rag = Rugst::new("memory.db")?;

    rag.remember(
        "test",
        "user",
        "user",
        "こんにちは、今日はRustを書いています",
    )?;

    let options = SearchOptions {
        top_k: 5,
        half_life_days: 30.0,
        min_score: 0.3,
        // None = チャンネルの全メッセージを検索対象にする(デフォルト挙動)。
        candidate_window: None,
        enable_fts: true,
        rrf_k: 60,
        fts_weight: 0.5,
    };

    let results = rag.search(
        "test",
        "user",
        "Rustについて話したこと",
        &options,
    )?;

    println!("検索結果:");
    for result in results {
        println!(
            "[{:.4}] {} ({})",
            result.score,
            result.text,
            result.created_at
        );
    }

    Ok(())
}