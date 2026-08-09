use crate::memory::MemoryCandidate;

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            top_k: 5,
            half_life_days: 30.0,
            min_score: 0.3,
        }
    }
}

pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || b.is_empty() {
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
}

#[derive(Debug)]
pub struct SearchResult<'a> {
    pub text: &'a str,
    pub score: f32,
    pub created_at: i64,
}

pub fn search_similar_with_decay<'a>(
    candidates: &'a [MemoryCandidate],
    query_embedding: &[f32],
    options: &SearchOptions,
) -> Vec<SearchResult<'a>> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    if options.half_life_days <= 0.0 {
        return Vec::new();
    }

    let mut scored: Vec<(&'a str, f32, i64)> = candidates
        .iter()
        .map(|candidate| {
            let similarity =
                cosine_similarity(&candidate.embedding, query_embedding);
            let age_days =
                ((now - candidate.created_at).max(0) as f32) / 86400.0;
            let decay =
                0.5_f32.powf(age_days / options.half_life_days);
            (
                candidate.text.as_str(),
                similarity * decay,
                candidate.created_at
            )
        })
        .filter(|(_, score, _)| *score >= options.min_score)
        .collect();

    scored.sort_by(|a, b|
        b.1.partial_cmp(&a.1).unwrap()
    );

    scored
        .into_iter()
        .take(options.top_k)
        .map(|(text, score, created_at)| SearchResult{
            text,
            score,
            created_at,
        })
        .collect()
}