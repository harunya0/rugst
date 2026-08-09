use crate::memory::MemoryCandidate;

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            top_k: 5,
            half_life_days: 30.0,
            min_score: 0.3,
            // None = チャンネルの全メッセージを検索対象にする。
            // 以前は呼び出し元(lib.rs)が1000件固定で先に絞り込んでおり、
            // 「意味的に近ければ古くても浮上する」というdecay設計の
            // 前提を実装が裏切っていた。デフォルトは全件対象に変更し、
            // 件数を絞りたい場合だけ明示的に Some(n) を指定する。
            candidate_window: None,
        }
    }
}

pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    // 次元数が異なるベクトル同士は比較不能。以前は zip が短い方に
    // 合わせて黙って内積を計算してしまい、埋め込みモデルを切り替えた
    // 際などに無意味なスコアを返す可能性があった。
    if a.is_empty() || b.is_empty() || a.len() != b.len() {
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
            // という意味に解釈する。以前は options.half_life_days <= 0.0 の
            // 場合、呼び出しごと空配列を返しており、「作られたばかりの
            // メッセージすら1件もヒットしない」という直感に反する挙動に
            // なっていた。作成直後(age_days == 0)は満点、それ以外は0とする。
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