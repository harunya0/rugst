use anyhow::Result;
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};

pub struct LocalEmbedding {
    model: TextEmbedding,
}

pub trait EmbeddingProvider {
    fn embed(&mut self, text: &str) -> Result<Vec<f32>>;
}

impl LocalEmbedding {
    pub fn new() -> Result<Self> {
        let model = TextEmbedding::try_new(
            InitOptions::new(EmbeddingModel::BGESmallENV15),
        )?;

        Ok(Self { model })
    }
}

impl EmbeddingProvider for LocalEmbedding {
    fn embed(&mut self, text: &str) -> Result<Vec<f32>> {
        let embeddings = self.model.embed(vec![text], None)?;

        embeddings
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("Embedding result is empty"))
    }
}