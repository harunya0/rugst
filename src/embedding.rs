use anyhow::Result;
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};

pub struct LocalEmbedding {
    model: TextEmbedding,
}

/// remember()で保存する本文(passage)と、search()の検索クエリ(query)を
/// 分けて埋め込めるようにするtrait。
///
/// MultilingualE5系のモデルは非対称検索(query/passageで役割が違う文章対)を
/// 前提に学習されており、"query: " / "passage: " のプレフィックスを
/// 付けたほうが検索精度が上がる。以前はBGESmallENV15(英語専用)を
/// prefixなしで使っていたため、日本語コンテンツでは埋め込み品質が
/// 落ちていた可能性がある。
pub trait EmbeddingProvider {
    /// 保存する本文(fact/会話ログなど)を埋め込む。
    fn embed_document(&mut self, text: &str) -> Result<Vec<f32>>;
    /// 検索クエリを埋め込む。
    fn embed_query(&mut self, text: &str) -> Result<Vec<f32>>;
}

impl LocalEmbedding {
    pub fn new() -> Result<Self> {
        // 多言語モデルに変更。日本語コンテンツ(文化祭Q&A)を扱うため、
        // 英語専用だったBGESmallENV15から差し替える。
        // 精度をさらに上げたい場合は MultilingualE5Base / Large も検討可(その分モデルサイズと推論コストは上がる)。
        let model = TextEmbedding::try_new(
            InitOptions::new(EmbeddingModel::MultilingualE5Small),
        )?;

        Ok(Self { model })
    }

    fn embed_with_prefix(&mut self, prefix: &str, text: &str) -> Result<Vec<f32>> {
        let prefixed = format!("{prefix}{text}");
        let embeddings = self.model.embed(vec![prefixed], None)?;

        embeddings
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("Embedding result is empty"))
    }
}

impl EmbeddingProvider for LocalEmbedding {
    fn embed_document(&mut self, text: &str) -> Result<Vec<f32>> {
        self.embed_with_prefix("passage: ", text)
    }

    fn embed_query(&mut self, text: &str) -> Result<Vec<f32>> {
        self.embed_with_prefix("query: ", text)
    }
}