use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};

use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::{bert, distilbert, qwen3};
use hf_hub::{
    Repo, RepoType,
    api::sync::{ApiBuilder, ApiRepo},
};
use serde::Deserialize;
use thiserror::Error;
use tokenizers::{Tokenizer, TruncationDirection, TruncationParams};

use crate::infrastructure::config::EmbeddingConfig;

#[derive(Debug, Error)]
pub enum EmbeddingError {
    #[error("model file access failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("model download failed: {0}")]
    Hub(#[from] hf_hub::api::sync::ApiError),
    #[error("model configuration failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("model inference failed: {0}")]
    Candle(#[from] candle_core::Error),
    #[error("tokenizer failed: {0}")]
    Tokenizer(String),
    #[error("model produced an empty embedding")]
    EmptyEmbedding,
    #[error("unsupported embedding backend: {0}")]
    Backend(String),
    #[error("unsupported embedding model architecture: {0:?}")]
    Architecture(Option<String>),
    #[error("embedding model returned {actual} vectors for {expected} documents")]
    VectorCount { expected: usize, actual: usize },
}

#[derive(Deserialize)]
struct ModelTypeProbe {
    model_type: Option<String>,
}

#[derive(Deserialize)]
struct MaxLengthProbe {
    max_position_embeddings: Option<usize>,
}

enum Backbone {
    Bert(bert::BertModel),
    Qwen3(qwen3::Model),
    DistilBert(distilbert::DistilBertModel),
}

pub struct Embedder {
    backbone: Backbone,
    tokenizer: Tokenizer,
    device: Device,
    batch_size: usize,
    truncation_limit: Option<usize>,
    truncated: AtomicBool,
    pub model_name: String,
    pub revision: String,
}

pub(crate) trait EmbeddingModel {
    fn model_name(&self) -> &str;
    fn revision(&self) -> &str;
    fn embed_query(&self, query: &str) -> Result<Vec<f32>, EmbeddingError>;
    fn embed_document(&self, document: &str) -> Result<Vec<f32>, EmbeddingError>;

    /// Embeds several documents, in input order. Models that can run a
    /// padded batch override this; the default embeds one at a time.
    fn embed_documents(&self, documents: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        documents
            .iter()
            .map(|document| self.embed_document(document))
            .collect()
    }
}

impl Embedder {
    pub fn load(config: &EmbeddingConfig) -> Result<Self, EmbeddingError> {
        let (device, backend) = select_device(&config.backend)?;
        tracing::info!(model = %config.model, revision = %config.revision, backend, "checking local embedding model cache");
        let repository = model_repository(config)?;
        if cached_model_files(config) {
            tracing::info!(model = %config.model, revision = %config.revision, "loading model from cache");
        } else {
            tracing::info!(model = %config.model, revision = %config.revision, "downloading model files");
        }
        let config_path = repository.get("config.json")?;
        let tokenizer_path = repository.get("tokenizer.json")?;
        tracing::info!(model = %config.model, revision = %config.revision, "loading embedding weights");
        let weights_path = repository.get("model.safetensors")?;
        let config_bytes = std::fs::read(config_path)?;
        let model_type = serde_json::from_slice::<ModelTypeProbe>(&config_bytes)?.model_type;
        let mut tokenizer = Tokenizer::from_file(tokenizer_path)
            .map_err(|error| EmbeddingError::Tokenizer(error.to_string()))?;
        let truncation_limit =
            serde_json::from_slice::<MaxLengthProbe>(&config_bytes)?.max_position_embeddings;
        if let Some(max_length) = truncation_limit {
            tokenizer
                .with_truncation(Some(TruncationParams {
                    max_length,
                    direction: TruncationDirection::Right,
                    ..Default::default()
                }))
                .map_err(|error| EmbeddingError::Tokenizer(error.to_string()))?;
        }
        let backbone = match model_type.as_deref() {
            None | Some("qwen3") => {
                let dtype = device.bf16_default_to_f32();
                let model_config = serde_json::from_slice::<qwen3::Config>(&config_bytes)?;
                let weights = unsafe {
                    VarBuilder::from_mmaped_safetensors(&[weights_path], dtype, &device)?
                }
                .rename_f(qwen_embedding_weight_name);
                Backbone::Qwen3(qwen3::Model::new(&model_config, weights)?)
            }
            Some("distilbert") => {
                // candle-transformers' distilbert attention casts scores to F32 for
                // softmax but matmuls the result against the still-BF16 value tensor,
                // so BF16 weights fail on CUDA (fine on CPU, where dtype is already
                // F32 throughout). Force F32 regardless of device to sidestep it.
                let model_config = serde_json::from_slice::<distilbert::Config>(&config_bytes)?;
                let weights = unsafe {
                    VarBuilder::from_mmaped_safetensors(&[weights_path], DType::F32, &device)?
                };
                Backbone::DistilBert(distilbert::DistilBertModel::load(weights, &model_config)?)
            }
            Some("bert") => {
                let model_config = serde_json::from_slice::<bert::Config>(&config_bytes)?;
                let weights = unsafe {
                    VarBuilder::from_mmaped_safetensors(&[weights_path], DType::F32, &device)?
                };
                Backbone::Bert(bert::BertModel::load(weights, &model_config)?)
            }
            Some(_) => return Err(EmbeddingError::Architecture(model_type)),
        };
        // Padded batches pay off on a GPU; on CPU the padding costs more than
        // batching saves, so default to one text per call there.
        let batch_size = config
            .batch_size
            .unwrap_or(if backend == "cuda" { 16 } else { 1 })
            .max(1);
        let embedder = Self {
            backbone,
            tokenizer,
            device,
            batch_size,
            truncation_limit,
            truncated: AtomicBool::new(false),
            model_name: config.model.clone(),
            revision: config.revision.clone(),
        };
        tracing::info!(model = %embedder.model_name, revision = %embedder.revision, backend, batch_size, "embedding model ready");
        Ok(embedder)
    }

    pub fn batch_size(&self) -> usize {
        self.batch_size
    }

    pub fn embed_query(&self, query: &str) -> Result<Vec<f32>, EmbeddingError> {
        match &self.backbone {
            Backbone::Qwen3(_) => self.embed_document(&format!(
                "Instruct: Given a memory request, retrieve the most relevant durable memory passages and relationship facts.\nQuery: {query}"
            )),
            Backbone::Bert(_) | Backbone::DistilBert(_) => self.embed_document(query),
        }
    }

    pub fn embed_document(&self, document: &str) -> Result<Vec<f32>, EmbeddingError> {
        let mut vectors = self.embed_documents(&[document])?;
        vectors.pop().ok_or(EmbeddingError::EmptyEmbedding)
    }

    /// Embeds `documents` in chunks of the configured batch size, so single
    /// and batched calls share one code path and produce the same vectors.
    pub fn embed_documents(&self, documents: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        let mut vectors = Vec::with_capacity(documents.len());
        for chunk in documents.chunks(self.batch_size) {
            match &self.backbone {
                // Qwen3 runs one text at a time; candle 0.11's
                // causal_mask cannot build a b>1 mask and has no padding mask.
                Backbone::Qwen3(model) => {
                    for document in chunk {
                        vectors.push(self.embed_qwen3(model, document)?);
                    }
                }
                Backbone::Bert(model) => vectors.extend(self.embed_bert(model, chunk)?),
                Backbone::DistilBert(model) => vectors.extend(self.embed_distilbert(model, chunk)?),
            }
        }
        Ok(vectors)
    }

    fn token_ids(&self, text: &str) -> Result<Vec<u32>, EmbeddingError> {
        let encoding = self
            .tokenizer
            .encode(text, true)
            .map_err(|error| EmbeddingError::Tokenizer(error.to_string()))?;
        if !encoding.get_overflowing().is_empty() {
            self.truncated.store(true, Ordering::Relaxed);
            tracing::warn!(
                model = %self.model_name,
                max_tokens = self.truncation_limit.unwrap_or(0),
                "embedding input exceeded the model's token limit and was truncated; content past the limit does not affect its embedding"
            );
        }
        let ids = encoding.get_ids();
        if ids.is_empty() {
            return Err(EmbeddingError::EmptyEmbedding);
        }
        Ok(ids.to_vec())
    }

    /// Returns and clears the truncation notice for recent embedding calls.
    pub fn take_truncation_warning(&self) -> Option<String> {
        if !self.truncated.swap(false, Ordering::Relaxed) {
            return None;
        }
        let limit = self.truncation_limit.unwrap_or(0);
        Some(format!(
            "one or more embedding inputs exceeded the model's {limit}-token limit and were \
             truncated; content beyond the first {limit} tokens does not affect embeddings"
        ))
    }

    fn embed_qwen3(&self, model: &qwen3::Model, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        let ids = self.token_ids(text)?;
        let input = Tensor::new(ids.as_slice(), &self.device)?.unsqueeze(0)?;
        let mut model = model.clone();
        let vector = model
            .forward(&input, 0)?
            .narrow(1, ids.len() - 1, 1)?
            .squeeze(1)?
            .squeeze(0)?
            .to_dtype(DType::F32)?
            .to_vec1::<f32>()?;
        normalize(vector)
    }

    fn embed_bert(
        &self,
        model: &bert::BertModel,
        texts: &[&str],
    ) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        let rows = texts
            .iter()
            .map(|text| self.token_ids(text))
            .collect::<Result<Vec<_>, _>>()?;
        let batch = rows.len();
        let length = rows.iter().map(Vec::len).max().unwrap_or(0);
        let mut ids = Vec::with_capacity(batch * length);
        let mut attention = Vec::with_capacity(batch * length);
        let mut real = Vec::with_capacity(batch * length);
        for row in &rows {
            for position in 0..length {
                let token = row.get(position);
                ids.push(token.copied().unwrap_or(0));
                attention.push(u8::from(token.is_some()));
                real.push(if token.is_some() { 1f32 } else { 0f32 });
            }
        }
        let input = Tensor::from_vec(ids, (batch, length), &self.device)?;
        let token_types = Tensor::zeros((batch, length), DType::U32, &self.device)?;
        let attention = Tensor::from_vec(attention, (batch, length), &self.device)?;
        let real = Tensor::from_vec(real, (batch, length, 1), &self.device)?;
        let pooled = model
            .forward(&input, &token_types, Some(&attention))?
            .to_dtype(DType::F32)?
            .broadcast_mul(&real)?
            .sum(1)?
            .broadcast_div(&real.sum(1)?)?;
        pooled
            .to_vec2::<f32>()?
            .into_iter()
            .map(normalize)
            .collect()
    }

    /// Right-pads the chunk, masks the padding out of attention, and
    /// mean-pools only the real tokens of each row.
    fn embed_distilbert(
        &self,
        model: &distilbert::DistilBertModel,
        texts: &[&str],
    ) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        let rows = texts
            .iter()
            .map(|text| self.token_ids(text))
            .collect::<Result<Vec<_>, _>>()?;
        let batch = rows.len();
        let length = rows.iter().map(Vec::len).max().unwrap_or(0);
        let mut ids = Vec::with_capacity(batch * length);
        let mut padding = Vec::with_capacity(batch * length);
        let mut real = Vec::with_capacity(batch * length);
        for row in &rows {
            for position in 0..length {
                let token = row.get(position);
                ids.push(token.copied().unwrap_or(0));
                padding.push(u8::from(token.is_none()));
                real.push(if token.is_some() { 1f32 } else { 0f32 });
            }
        }
        let input = Tensor::from_vec(ids, (batch, length), &self.device)?;
        // Non-zero mask entries become -inf attention scores (padding keys).
        let mask = Tensor::from_vec(padding, (batch, 1, 1, length), &self.device)?;
        let real = Tensor::from_vec(real, (batch, length, 1), &self.device)?;
        let pooled = model
            .forward(&input, &mask)?
            .to_dtype(DType::F32)?
            .broadcast_mul(&real)?
            .sum(1)?
            .broadcast_div(&real.sum(1)?)?;
        pooled
            .to_vec2::<f32>()?
            .into_iter()
            .map(normalize)
            .collect()
    }
}

fn normalize(vector: Vec<f32>) -> Result<Vec<f32>, EmbeddingError> {
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm == 0.0 || !norm.is_finite() {
        return Err(EmbeddingError::EmptyEmbedding);
    }
    Ok(vector.into_iter().map(|value| value / norm).collect())
}

fn model_repository(config: &EmbeddingConfig) -> Result<ApiRepo, EmbeddingError> {
    let api = ApiBuilder::new()
        .with_cache_dir(config.cache_dir.clone())
        .with_progress(false)
        .build()?;
    Ok(api.repo(Repo::with_revision(
        config.model.clone(),
        RepoType::Model,
        config.revision.clone(),
    )))
}

/// The active embedding configuration with the exact token limit read from
/// the checkpoint's `config.json`. Reading this fetches (or reads from the
/// local cache) only `config.json`, not the model weights.
pub struct EmbeddingDetails {
    pub enabled: bool,
    pub model: String,
    pub revision: String,
    pub max_tokens: Option<usize>,
}

pub fn embedding_details(config: &EmbeddingConfig) -> Result<EmbeddingDetails, EmbeddingError> {
    let mut details = EmbeddingDetails {
        enabled: config.enabled,
        model: config.model.clone(),
        revision: config.revision.clone(),
        max_tokens: None,
    };
    if !config.enabled {
        return Ok(details);
    }
    let repository = model_repository(config)?;
    let config_path = repository.get("config.json")?;
    let config_bytes = std::fs::read(config_path)?;
    details.max_tokens =
        serde_json::from_slice::<MaxLengthProbe>(&config_bytes)?.max_position_embeddings;
    Ok(details)
}

fn cached_model_files(config: &EmbeddingConfig) -> bool {
    let repository = config
        .cache_dir
        .join(format!("models--{}", config.model.replace('/', "--")));
    let revision = match fs::read_to_string(repository.join("refs").join(&config.revision)) {
        Ok(revision) => revision.trim().to_owned(),
        Err(_) => return false,
    };
    let snapshot = repository.join("snapshots").join(revision);
    ["config.json", "tokenizer.json", "model.safetensors"]
        .iter()
        .map(|file| snapshot.join(file))
        .all(|file| file.is_file())
}

fn select_device(requested: &str) -> Result<(Device, &'static str), EmbeddingError> {
    match requested.trim().to_lowercase().as_str() {
        "cpu" => Ok((Device::Cpu, "cpu")),
        "auto" => match Device::cuda_if_available(0) {
            Ok(Device::Cuda(device)) => Ok((Device::Cuda(device), "cuda")),
            _ => Ok((Device::Cpu, "cpu")),
        },
        "cuda" => Device::new_cuda(0)
            .map(|device| (device, "cuda"))
            .map_err(|error| EmbeddingError::Backend(error.to_string())),
        backend => Err(EmbeddingError::Backend(backend.to_owned())),
    }
}

fn qwen_embedding_weight_name(name: &str) -> String {
    name.strip_prefix("model.").unwrap_or(name).to_owned()
}

impl EmbeddingModel for Embedder {
    fn model_name(&self) -> &str {
        &self.model_name
    }

    fn revision(&self) -> &str {
        &self.revision
    }

    fn embed_query(&self, query: &str) -> Result<Vec<f32>, EmbeddingError> {
        Embedder::embed_query(self, query)
    }

    fn embed_document(&self, document: &str) -> Result<Vec<f32>, EmbeddingError> {
        Embedder::embed_document(self, document)
    }

    fn embed_documents(&self, documents: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        Embedder::embed_documents(self, documents)
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use std::path::PathBuf;

    use super::{Embedder, embedding_details, qwen_embedding_weight_name};
    use crate::infrastructure::config::{ConfigOverrides, EmbeddingConfig, embedding_config};

    /// Loads the configured model from `GRAPHMEM_EMBEDDING_CACHE_DIR`, or the
    /// repository's `.data/models` cache used by the eval script.
    #[test]
    #[ignore = "downloads and runs the real embedding model"]
    fn all_minilm_batch_matches_one_at_a_time_embedding() {
        let data_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(".data");
        let mut config = embedding_config(&data_dir, &ConfigOverrides::default()).unwrap();
        config.model = "sentence-transformers/all-MiniLM-L6-v2".to_owned();
        config.backend = "cpu".to_owned();
        config.batch_size = Some(3);
        let embedder = Embedder::load(&config).unwrap();
        let documents = [
            "short",
            "a noticeably longer document so the batch needs padding tokens",
            "retry queue",
            "the fourth document lands in a second chunk",
        ];
        let batched = embedder.embed_documents(&documents).unwrap();
        assert_eq!(batched.len(), documents.len());
        for (document, vector) in documents.iter().zip(&batched) {
            assert_eq!(vector.len(), 384);
            let single = embedder.embed_document(document).unwrap();
            let cosine: f32 = single.iter().zip(vector).map(|(a, b)| a * b).sum();
            assert!(cosine > 0.999, "{document}: cosine {cosine}");
        }
        assert!(embedder.take_truncation_warning().is_none());
        let long = "token ".repeat(600);
        embedder.embed_document(&long).unwrap();
        let warning = embedder
            .take_truncation_warning()
            .expect("long input warns about truncation");
        assert!(warning.contains("512"), "{warning}");
        assert!(embedder.take_truncation_warning().is_none());
    }

    #[test]
    fn disabled_embeddings_report_no_token_limit_without_fetching() {
        let config = EmbeddingConfig {
            enabled: false,
            model: "some/model".to_owned(),
            revision: "main".to_owned(),
            cache_dir: PathBuf::from("/nonexistent"),
            backend: "cpu".to_owned(),
            batch_size: None,
        };
        let details = embedding_details(&config).unwrap();
        assert!(!details.enabled);
        assert_eq!(details.model, "some/model");
        assert_eq!(details.max_tokens, None);
    }

    #[test]
    fn maps_candle_qwen_names_to_embedding_checkpoint_names() {
        assert_eq!(
            qwen_embedding_weight_name("model.embed_tokens.weight"),
            "embed_tokens.weight"
        );
        assert_eq!(
            qwen_embedding_weight_name("model.layers.0.self_attn.q_proj.weight"),
            "layers.0.self_attn.q_proj.weight"
        );
        assert_eq!(
            qwen_embedding_weight_name("lm_head.weight"),
            "lm_head.weight"
        );
    }
}
