use crate::memory::MemoryCandidate;

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            top_k: 5,
            half_life_days: 30.0,
            min_score: 0.3,
            // None = チャンネルの全メッセージを検索対象にする。
            candidate_window: None,
        }
    }
}

pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }

    // 次元数が異なるベクトル同士は比較不能。以前は zip が短い方に
    // 合わせて黙って内積を計算していたため、埋め込みモデルを切り替えた
    // 際などに気づかれないまま無意味なスコアを返す恐れがあった。
    // 0.0を返すだけでなく、原因調査の手がかりとして警告も出す。
    if a.len() != b.len() {
        eprintln!(
            "rugst: embedding dimension mismatch ({} vs {}); \
             returning similarity 0.0. モデルを変更した場合は \
             古いDBのembeddingを再生成してください。",
            a.len(),
            b.len()
        );
        return 0.0;
    }

    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}

#[derive(Debug, Clone)]
pub struct SearchOptions {
    pub top_k: usize,
    pub half_life_days: f32,
    pub min_score: f32,
    /// 類似度計算の対象にする候補の件数(直近何件から選ぶか)。
    /// `None` ならチャンネルの全メッセージが対象。
    pub candidate_window: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub text: String,
    pub score: f32,
    pub created_at: i64,
}

pub fn search_similar_with_decay<'a>(
    candidates: &'a [MemoryCandidate],
    query_embedding: &[f32],
    options: &SearchOptions,
) -> Vec<SearchResult> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    let mut scored: Vec<(&'a str, f32, i64)> = candidates
        .iter()
        .map(|candidate| {
            let similarity =
                cosine_similarity(&candidate.embedding, query_embedding);
            let age_days =
                ((now - candidate.created_at).max(0) as f32) / 86400.0;

            // half_life_days <= 0 は「半減期が存在しない=即座に無関係になる」
            // という意味に解釈する。作成直後(age_days == 0)は満点、
            // それ以外は0とする。
            let decay = if options.half_life_days > 0.0 {
                0.5_f32.powf(age_days / options.half_life_days)
            } else if age_days <= 0.0 {
                1.0
            } else {
                0.0
            };

            (
                candidate.text.as_str(),
                similarity * decay,
                candidate.created_at
            )
        })
        .filter(|(_, score, _)| *score >= options.min_score)
        .collect();

    // NaNが混入していてもpanicせず、順序不定のまま扱う
    scored.sort_by(|a, b|
        b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
    );

    scored
        .into_iter()
        .take(options.top_k)
        .map(|(text, score, created_at)| SearchResult{
            text: text.to_string(),
            score,
            created_at,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_similarity_identical_vectors_is_one() {
        let v = vec![1.0, 2.0, 3.0];
        let sim = cosine_similarity(&v, &v);
        assert!((sim - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_similarity_orthogonal_vectors_is_zero() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        let sim = cosine_similarity(&a, &b);
        assert!(sim.abs() < 1e-6);
    }

    #[test]
    fn cosine_similarity_dimension_mismatch_returns_zero() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![1.0, 2.0];
        assert_eq!(cosine_similarity(&a, &b), 0.0);
    }

    #[test]
    fn cosine_similarity_empty_vector_returns_zero() {
        let a: Vec<f32> = vec![];
        let b = vec![1.0, 2.0];
        assert_eq!(cosine_similarity(&a, &b), 0.0);
    }

    fn candidate(text: &str, embedding: Vec<f32>, age_days: f32) -> MemoryCandidate {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        MemoryCandidate {
            text: text.to_string(),
            embedding,
            created_at: now - (age_days * 86400.0) as i64,
        }
    }

    #[test]
    fn search_similar_with_decay_prefers_recent_over_old_with_same_similarity() {
        let query = vec![1.0, 0.0];
        let candidates = vec![
            candidate("old", vec![1.0, 0.0], 60.0),
            candidate("recent", vec![1.0, 0.0], 0.0),
        ];
        let options = SearchOptions {
            top_k: 2,
            half_life_days: 30.0,
            min_score: 0.0,
            candidate_window: None,
        };

        let results = search_similar_with_decay(&candidates, &query, &options);

        assert_eq!(results[0].text, "recent");
        assert_eq!(results[1].text, "old");
        assert!(results[0].score > results[1].score);
    }

    #[test]
    fn search_similar_with_decay_half_life_zero_keeps_only_brand_new_items() {
        let query = vec![1.0, 0.0];
        let candidates = vec![
            candidate("just_now", vec![1.0, 0.0], 0.0),
            candidate("yesterday", vec![1.0, 0.0], 1.0),
        ];
        let options = SearchOptions {
            top_k: 5,
            half_life_days: 0.0,
            min_score: 0.01,
            candidate_window: None,
        };

        let results = search_similar_with_decay(&candidates, &query, &options);

        // half_life<=0 は「即座に無関係になる」という意味なので、
        // 作成直後(age=0)のものだけが残る。以前はhalf_life<=0の場合
        // 呼び出しごと空配列を返しており、直感に反する挙動だった。
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].text, "just_now");
    }

    #[test]
    fn search_similar_with_decay_respects_min_score() {
        let query = vec![1.0, 0.0];
        let candidates = vec![candidate("orthogonal", vec![0.0, 1.0], 0.0)];
        let options = SearchOptions {
            top_k: 5,
            half_life_days: 30.0,
            min_score: 0.5,
            candidate_window: None,
        };

        let results = search_similar_with_decay(&candidates, &query, &options);
        assert!(results.is_empty());
    }
}