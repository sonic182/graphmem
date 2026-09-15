use std::fs;

use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::{distilbert, qwen3};
use hf_hub::{Repo, RepoType, api::sync::ApiBuilder};
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
    Qwen3(qwen3::Model),
    DistilBert(distilbert::DistilBertModel),
}

pub struct Embedder {
    backbone: Backbone,
    tokenizer: Tokenizer,
    device: Device,
    pub model_name: String,
    pub revision: String,
}

pub(crate) trait EmbeddingModel {
    fn model_name(&self) -> &str;
    fn revision(&self) -> &str;
    fn embed_query(&self, query: &str) -> Result<Vec<f32>, EmbeddingError>;
    fn embed_document(&self, document: &str) -> Result<Vec<f32>, EmbeddingError>;
}

impl Embedder {
    pub fn load(config: &EmbeddingConfig) -> Result<Self, EmbeddingError> {
        let (device, backend) = select_device(&config.backend)?;
        tracing::info!(model = %config.model, revision = %config.revision, backend, "checking local embedding model cache");
        let api = ApiBuilder::new()
            .with_cache_dir(config.cache_dir.clone())
            .with_progress(false)
            .build()?;
        let repository = api.repo(Repo::with_revision(
            config.model.clone(),
            RepoType::Model,
            config.revision.clone(),
        ));
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
        if let Some(max_length) =
            serde_json::from_slice::<MaxLengthProbe>(&config_bytes)?.max_position_embeddings
        {
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
            Some(_) => return Err(EmbeddingError::Architecture(model_type)),
        };
        let embedder = Self {
            backbone,
            tokenizer,
            device,
            model_name: config.model.clone(),
            revision: config.revision.clone(),
        };
        tracing::info!(model = %embedder.model_name, revision = %embedder.revision, backend, "embedding model ready");
        Ok(embedder)
    }

    pub fn embed_query(&self, query: &str) -> Result<Vec<f32>, EmbeddingError> {
        match &self.backbone {
            Backbone::Qwen3(_) => self.embed(&format!(
                "Instruct: Given a memory request, retrieve the most relevant durable memory passages and relationship facts.\nQuery: {query}"
            )),
            Backbone::DistilBert(_) => self.embed(query),
        }
    }

    pub fn embed_document(&self, document: &str) -> Result<Vec<f32>, EmbeddingError> {
        self.embed(document)
    }

    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        let encoding = self
            .tokenizer
            .encode(text, true)
            .map_err(|error| EmbeddingError::Tokenizer(error.to_string()))?;
        let ids = encoding.get_ids();
        if ids.is_empty() {
            return Err(EmbeddingError::EmptyEmbedding);
        }
        let input = Tensor::new(ids, &self.device)?.unsqueeze(0)?;
        let vector = match &self.backbone {
            Backbone::Qwen3(model) => {
                let mut model = model.clone();
                model
                    .forward(&input, 0)?
                    .narrow(1, ids.len() - 1, 1)?
                    .squeeze(1)?
                    .squeeze(0)?
                    .to_dtype(DType::F32)?
            }
            Backbone::DistilBert(model) => {
                let mask = Tensor::zeros((ids.len(), ids.len()), DType::U8, &self.device)?;
                model
                    .forward(&input, &mask)?
                    .mean(1)?
                    .squeeze(0)?
                    .to_dtype(DType::F32)?
            }
        };
        let norm = vector.norm()?.to_scalar::<f32>()?;
        if norm == 0.0 {
            return Err(EmbeddingError::EmptyEmbedding);
        }
        Ok((&vector / norm as f64)?.to_vec1::<f32>()?)
    }
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
}

#[cfg(test)]
mod tests {
    use super::qwen_embedding_weight_name;

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
