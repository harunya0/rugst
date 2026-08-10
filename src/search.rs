use crate::memory::MemoryCandidate;

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            top_k: 5,
            half_life_days: 30.0,
            min_score: 0.3,
            // None = チャンネルの全メッセージを検索対象にする。
            candidate_window: None,
            enable_fts: false,
            rrf_k: 60,
            fts_weight: 1.0,
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
    /// ハイブリッド検索(ベクトル類似度 + FTS5のBM25キーワード検索をRRFで統合)
    /// を有効にするか。falseなら従来通りベクトル検索のみ。
    pub enable_fts: bool,
    /// RRF(Reciprocal Rank Fusion)のkパラメータ。値が大きいほど下位の順位の
    /// 影響が均される(=上位と下位の差がつきにくくなる)。文献でよく使われる
    /// 既定値は60。
    pub rrf_k: u32,
    /// FTS5側のRRFスコアに掛ける重み。1.0がベクトル側と対等、大きいほど
    /// キーワード一致(FTS5)を、小さいほど意味的類似度(ベクトル)を重視する。
    pub fts_weight: f32,
}

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub id: i64,
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

    let mut scored: Vec<(i64, &'a str, f32, i64)> = candidates
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
                candidate.id,
                candidate.text.as_str(),
                similarity * decay,
                candidate.created_at
            )
        })
        .filter(|(_, _, score, _)| *score >= options.min_score)
        .collect();

    // NaNが混入していてもpanicせず、順序不定のまま扱う
    scored.sort_by(|a, b|
        b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal)
    );

    scored
        .into_iter()
        .take(options.top_k)
        .map(|(id, text, score, created_at)| SearchResult{
            id,
            text: text.to_string(),
            score,
            created_at,
        })
        .collect()
}

/// ベクトル検索とFTS5(BM25)検索をRRFで統合したハイブリッド検索。
///
/// `vector_candidates` はここでコサイン類似度によりランキングし直すので、
/// 呼び出し側で事前にソートされている必要はない。
/// `fts_candidates` は `HistoryStore::get_fts_candidates` が返す順序
/// (bm25順=一致度が高い順)のまま渡すこと。
pub fn search_hybrid(
    vector_candidates: &[MemoryCandidate],
    fts_candidates: &[MemoryCandidate],
    query_embedding: &[f32],
    options: &SearchOptions,
) -> Vec<SearchResult> {
    let mut vector_scored: Vec<(&MemoryCandidate, f32)> = vector_candidates
        .iter()
        .map(|c| (c, cosine_similarity(&c.embedding, query_embedding)))
        .collect();
    vector_scored.sort_by(|a, b| {
        b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
    });
    let vector_ranked: Vec<&MemoryCandidate> =
        vector_scored.into_iter().map(|(c, _)| c).collect();

    let fts_ranked: Vec<&MemoryCandidate> = fts_candidates.iter().collect();

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    let mut results = reciprocal_rank_fusion(
        &vector_ranked,
        &fts_ranked,
        options.rrf_k,
        options.fts_weight,
        options.half_life_days,
        now,
    );

    // RRFスコアはcosine類似度(0〜1)と全くスケールが異なる(rrf_k=60なら
    // 1位でも最大 1/61 前後)ため、reciprocal_rank_fusion内で0〜1に
    // 正規化した上でここでmin_scoreによる足切りを行う。
    // 以前はここでmin_scoreを一切見ておらず、enable_fts=trueのときだけ
    // min_scoreが事実上無視される(無関係な候補まで返る)状態になっていた。
    results.retain(|r| r.score >= options.min_score);

    results.truncate(options.top_k);
    results
}

/// RRF(Reciprocal Rank Fusion)でベクトル検索とFTS5検索の順位を統合する。
/// `1 / (k + rank)` の値をリストごとに計算し、同じレコード(id)について
/// 合算する。スコアの絶対値ではなく「順位」だけを使うため、コサイン類似度
/// (0〜1、大きいほど良い)とBM25(sqliteの実装では0以下、小さいほど良い)の
/// ようにスケールも向きも異なる指標同士を公平に合成できる。
///
/// 時間減衰は各リストの順位付けそのものには使わず、RRFで統合した後の
/// 最終スコアに一括で掛ける。ベクトル側・FTS側それぞれの順位付けの時点で
/// 個別に減衰を掛けると、両方に一致した新しめのレコードが二重に優遇されたり
/// 逆に片方でしか順位が動かず調整が難しくなったりするため、統合後に
/// 一箇所でだけ「新しさ」を反映させたほうが挙動を把握しやすい。
fn reciprocal_rank_fusion<'a>(
    vector_ranked: &[&'a MemoryCandidate],
    fts_ranked: &[&'a MemoryCandidate],
    rrf_k: u32,
    fts_weight: f32,
    half_life_days: f32,
    now: i64,
) -> Vec<SearchResult> {
    use std::collections::HashMap;

    let mut scores: HashMap<i64, (f32, &'a str, i64)> = HashMap::new();

    for (rank, candidate) in vector_ranked.iter().enumerate() {
        let rrf_score = 1.0 / (rrf_k as f32 + rank as f32 + 1.0);
        let entry = scores
            .entry(candidate.id)
            .or_insert((0.0, candidate.text.as_str(), candidate.created_at));
        entry.0 += rrf_score;
    }

    for (rank, candidate) in fts_ranked.iter().enumerate() {
        let rrf_score = fts_weight / (rrf_k as f32 + rank as f32 + 1.0);
        let entry = scores
            .entry(candidate.id)
            .or_insert((0.0, candidate.text.as_str(), candidate.created_at));
        entry.0 += rrf_score;
    }

    // 両方のリストで1位を取った場合(=理論上の最高スコア)を1.0として
    // 正規化する。これにより呼び出し側のmin_scoreがコサイン類似度と
    // 同じ0〜1のスケールで意味を持つようになる。
    let max_possible_rrf = (1.0 + fts_weight) / (rrf_k as f32 + 1.0);

    let mut results: Vec<SearchResult> = scores
        .into_iter()
        .map(|(id, (rrf_score, text, created_at))| {
            let age_days = ((now - created_at).max(0) as f32) / 86400.0;
            let decay = if half_life_days > 0.0 {
                0.5_f32.powf(age_days / half_life_days)
            } else if age_days <= 0.0 {
                1.0
            } else {
                0.0
            };

            let normalized = if max_possible_rrf > 0.0 {
                rrf_score / max_possible_rrf
            } else {
                0.0
            };

            SearchResult {
                id,
                text: text.to_string(),
                score: normalized * decay,
                created_at,
            }
        })
        .collect();

    // NaNが混入していてもpanicせず、順序不定のまま扱う
    results.sort_by(|a, b| {
        b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal)
    });

    results
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

    fn candidate(id: i64, text: &str, embedding: Vec<f32>, age_days: f32) -> MemoryCandidate {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        MemoryCandidate {
            id,
            text: text.to_string(),
            embedding,
            created_at: now - (age_days * 86400.0) as i64,
        }
    }

    #[test]
    fn search_similar_with_decay_prefers_recent_over_old_with_same_similarity() {
        let query = vec![1.0, 0.0];
        let candidates = vec![
            candidate(1, "old", vec![1.0, 0.0], 60.0),
            candidate(2, "recent", vec![1.0, 0.0], 0.0),
        ];
        let options = SearchOptions {
            top_k: 2,
            half_life_days: 30.0,
            min_score: 0.0,
            candidate_window: None,
            enable_fts: false,
            rrf_k: 60,
            fts_weight: 1.0,
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
            candidate(1, "just_now", vec![1.0, 0.0], 0.0),
            candidate(2, "yesterday", vec![1.0, 0.0], 1.0),
        ];
        let options = SearchOptions {
            top_k: 5,
            half_life_days: 0.0,
            min_score: 0.01,
            candidate_window: None,
            enable_fts: false,
            rrf_k: 60,
            fts_weight: 1.0,
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
        let candidates = vec![candidate(1, "orthogonal", vec![0.0, 1.0], 0.0)];
        let options = SearchOptions {
            top_k: 5,
            half_life_days: 30.0,
            min_score: 0.5,
            candidate_window: None,
            enable_fts: false,
            rrf_k: 60,
            fts_weight: 1.0,
        };

        let results = search_similar_with_decay(&candidates, &query, &options);
        assert!(results.is_empty());
    }

    #[test]
    fn search_hybrid_merges_vector_only_and_fts_only_hits() {
        // vector側にしか無いid=1、fts側にしか無いid=2、両方にヒットするid=3
        // という状況で、3つとも結果に出ることを確認する。
        let query = vec![1.0, 0.0];
        let vector_candidates = vec![
            candidate(1, "vector_only", vec![1.0, 0.0], 0.0),
            candidate(3, "both", vec![0.9, 0.1], 0.0),
        ];
        let fts_candidates = vec![
            candidate(2, "fts_only", vec![], 0.0),
            candidate(3, "both", vec![], 0.0),
        ];
        let options = SearchOptions {
            top_k: 5,
            half_life_days: 30.0,
            min_score: 0.0,
            candidate_window: None,
            enable_fts: true,
            rrf_k: 60,
            fts_weight: 1.0,
        };

        let results = search_hybrid(&vector_candidates, &fts_candidates, &query, &options);
        let ids: Vec<i64> = results.iter().map(|r| r.id).collect();

        assert_eq!(ids.len(), 3);
        assert!(ids.contains(&1));
        assert!(ids.contains(&2));
        assert!(ids.contains(&3));
    }

    #[test]
    fn search_hybrid_ranks_hit_in_both_lists_above_single_list_hits() {
        // 両方のリストで1位のid=3は、片方でしかヒットしないid=1/2より
        // RRFスコアが高くなるはず。
        let query = vec![1.0, 0.0];
        let vector_candidates = vec![
            candidate(3, "both", vec![1.0, 0.0], 0.0),
            candidate(1, "vector_only", vec![0.99, 0.01], 0.0),
        ];
        let fts_candidates = vec![
            candidate(3, "both", vec![], 0.0),
            candidate(2, "fts_only", vec![], 0.0),
        ];
        let options = SearchOptions {
            top_k: 5,
            half_life_days: 30.0,
            min_score: 0.0,
            candidate_window: None,
            enable_fts: true,
            rrf_k: 60,
            fts_weight: 1.0,
        };

        let results = search_hybrid(&vector_candidates, &fts_candidates, &query, &options);

        assert_eq!(results[0].id, 3);
    }

    #[test]
    fn search_hybrid_respects_min_score() {
        // fts_weightを小さくすると、FTS側のみで1位を取った候補でも
        // 正規化後のRRFスコアの上限が fts_weight/(1+fts_weight) に抑えられる。
        // ここでは0.1/(1+0.1) ≈ 0.0909 < min_score(0.3) となるはずなので、
        // vector側にヒットが無いこの候補は足切りされて除外される。
        let query = vec![1.0, 0.0];
        let fts_candidates = vec![candidate(1, "weak_match", vec![], 0.0)];
        let options = SearchOptions {
            top_k: 5,
            half_life_days: 30.0,
            min_score: 0.3,
            candidate_window: None,
            enable_fts: true,
            rrf_k: 60,
            fts_weight: 0.1,
        };

        let results = search_hybrid(&[], &fts_candidates, &query, &options);
        assert!(results.is_empty());
    }

    #[test]
    fn search_hybrid_top_rank_in_both_lists_passes_min_score() {
        // 両方のリストで1位を取れば正規化スコアは1.0なので、
        // 現実的なmin_scoreは通過するはず。
        let query = vec![1.0, 0.0];
        let vector_candidates = vec![candidate(1, "both", vec![1.0, 0.0], 0.0)];
        let fts_candidates = vec![candidate(1, "both", vec![], 0.0)];
        let options = SearchOptions {
            top_k: 5,
            half_life_days: 30.0,
            min_score: 0.9,
            candidate_window: None,
            enable_fts: true,
            rrf_k: 60,
            fts_weight: 0.5,
        };

        let results = search_hybrid(&vector_candidates, &fts_candidates, &query, &options);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn search_hybrid_respects_top_k() {
        let query = vec![1.0, 0.0];
        let vector_candidates = vec![
            candidate(1, "a", vec![1.0, 0.0], 0.0),
            candidate(2, "b", vec![1.0, 0.0], 0.0),
            candidate(3, "c", vec![1.0, 0.0], 0.0),
        ];
        let options = SearchOptions {
            top_k: 2,
            half_life_days: 30.0,
            min_score: 0.0,
            candidate_window: None,
            enable_fts: true,
            rrf_k: 60,
            fts_weight: 1.0,
        };

        let results = search_hybrid(&vector_candidates, &[], &query, &options);
        assert_eq!(results.len(), 2);
    }
}