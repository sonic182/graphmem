use std::{
    env, fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use thiserror::Error;

const DEFAULT_MODEL: &str = "sentence-transformers/msmarco-MiniLM-L6-cos-v5";
const DEFAULT_REVISION: &str = "main";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddingConfig {
    pub enabled: bool,
    pub model: String,
    pub revision: String,
    pub cache_dir: PathBuf,
    pub backend: String,
    /// Texts per model call; `None` lets the embedder pick one for its device.
    pub batch_size: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RetrievalConfig {
    pub seed_top_k: usize,
    pub seed_temperature: f64,
    pub memory_seed_weight: f64,
    pub entity_anchor_weight: f64,
    pub damping: f64,
}

impl Default for RetrievalConfig {
    fn default() -> Self {
        Self {
            seed_top_k: 20,
            seed_temperature: 0.05,
            memory_seed_weight: 0.5,
            entity_anchor_weight: 0.2,
            damping: 0.5,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeConfig {
    pub worker_threads: usize,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("configuration error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid configuration: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("invalid configuration: {0}")]
    Invalid(String),
}

#[derive(Debug, Default, Deserialize)]
struct FileConfig {
    embedding: Option<FileEmbeddingConfig>,
    retrieval: Option<FileRetrievalConfig>,
    runtime: Option<FileRuntimeConfig>,
}

#[derive(Debug, Default, Deserialize)]
struct FileEmbeddingConfig {
    enabled: Option<bool>,
    model: Option<String>,
    revision: Option<String>,
    cache_dir: Option<PathBuf>,
    backend: Option<String>,
    batch_size: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
struct FileRetrievalConfig {
    seed_top_k: Option<usize>,
    seed_temperature: Option<f64>,
    memory_seed_weight: Option<f64>,
    entity_anchor_weight: Option<f64>,
    damping: Option<f64>,
}

#[derive(Debug, Default, Deserialize)]
struct FileRuntimeConfig {
    worker_threads: Option<usize>,
}

fn read_file_config(data_dir: &Path) -> Result<FileConfig, ConfigError> {
    match fs::read_to_string(data_dir.join("config.toml")) {
        Ok(contents) => Ok(toml::from_str::<FileConfig>(&contents)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(FileConfig::default()),
        Err(error) => Err(error.into()),
    }
}

pub fn embedding_config(data_dir: &Path) -> Result<EmbeddingConfig, ConfigError> {
    let embedding = read_file_config(data_dir)?.embedding.unwrap_or_default();
    let batch_size = env_number("GRAPHMEM_EMBEDDING_BATCH_SIZE")?.or(embedding.batch_size);
    if let Some(batch_size) = batch_size {
        validate_batch_size(batch_size)?;
    }
    Ok(EmbeddingConfig {
        enabled: env::var("GRAPHMEM_EMBEDDINGS")
            .ok()
            .map(|value| value != "off")
            .or(embedding.enabled)
            .unwrap_or(true),
        model: env_value("GRAPHMEM_EMBEDDING_MODEL")
            .or(embedding.model)
            .unwrap_or_else(|| DEFAULT_MODEL.to_owned()),
        revision: env_value("GRAPHMEM_EMBEDDING_REVISION")
            .or(embedding.revision)
            .unwrap_or_else(|| DEFAULT_REVISION.to_owned()),
        cache_dir: env::var_os("GRAPHMEM_EMBEDDING_CACHE_DIR")
            .map(PathBuf::from)
            .or(embedding.cache_dir)
            .unwrap_or_else(|| data_dir.join("models")),
        backend: env_value("GRAPHMEM_EMBEDDING_BACKEND")
            .or(embedding.backend)
            .unwrap_or_else(|| "auto".to_owned()),
        batch_size,
    })
}

pub fn validate_batch_size(batch_size: usize) -> Result<(), ConfigError> {
    if batch_size == 0 {
        return Err(ConfigError::Invalid(
            "embedding.batch_size must be at least 1".to_owned(),
        ));
    }
    Ok(())
}

pub fn retrieval_config(data_dir: &Path) -> Result<RetrievalConfig, ConfigError> {
    let file = read_file_config(data_dir)?.retrieval.unwrap_or_default();
    let defaults = RetrievalConfig::default();
    let config = RetrievalConfig {
        seed_top_k: env_number("GRAPHMEM_RETRIEVAL_SEED_TOP_K")?
            .or(file.seed_top_k)
            .unwrap_or(defaults.seed_top_k),
        seed_temperature: env_number("GRAPHMEM_RETRIEVAL_SEED_TEMPERATURE")?
            .or(file.seed_temperature)
            .unwrap_or(defaults.seed_temperature),
        memory_seed_weight: env_number("GRAPHMEM_RETRIEVAL_MEMORY_SEED_WEIGHT")?
            .or(file.memory_seed_weight)
            .unwrap_or(defaults.memory_seed_weight),
        entity_anchor_weight: env_number("GRAPHMEM_RETRIEVAL_ENTITY_ANCHOR_WEIGHT")?
            .or(file.entity_anchor_weight)
            .unwrap_or(defaults.entity_anchor_weight),
        damping: env_number("GRAPHMEM_RETRIEVAL_DAMPING")?
            .or(file.damping)
            .unwrap_or(defaults.damping),
    };
    if config.seed_top_k == 0 {
        return Err(ConfigError::Invalid(
            "retrieval.seed_top_k must be at least 1".to_owned(),
        ));
    }
    if !(config.seed_temperature > 0.0 && config.seed_temperature <= 1.0) {
        return Err(ConfigError::Invalid(
            "retrieval.seed_temperature must be between 0 and 1".to_owned(),
        ));
    }
    if !(0.0..=1.0).contains(&config.memory_seed_weight) {
        return Err(ConfigError::Invalid(
            "retrieval.memory_seed_weight must be between 0 and 1".to_owned(),
        ));
    }
    if !(0.0..=1.0).contains(&config.entity_anchor_weight) {
        return Err(ConfigError::Invalid(
            "retrieval.entity_anchor_weight must be between 0 and 1".to_owned(),
        ));
    }
    if !(config.damping > 0.0 && config.damping < 1.0) {
        return Err(ConfigError::Invalid(
            "retrieval.damping must be between 0 and 1, exclusive".to_owned(),
        ));
    }
    Ok(config)
}

pub fn runtime_config(data_dir: &Path) -> Result<RuntimeConfig, ConfigError> {
    let file = read_file_config(data_dir)?;
    let worker_threads = match env::var("GRAPHMEM_TOKIO_WORKER_THREADS") {
        Ok(value) => value.parse::<usize>().map_err(|_| {
            ConfigError::Invalid(
                "GRAPHMEM_TOKIO_WORKER_THREADS must be a positive integer".to_owned(),
            )
        })?,
        Err(_) => file
            .runtime
            .and_then(|runtime| runtime.worker_threads)
            .unwrap_or(4),
    };
    if worker_threads == 0 || worker_threads > 64 {
        return Err(ConfigError::Invalid(
            "runtime.worker_threads must be between 1 and 64".to_owned(),
        ));
    }
    Ok(RuntimeConfig { worker_threads })
}

fn env_value(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.trim().is_empty())
}

fn env_number<T: std::str::FromStr>(name: &str) -> Result<Option<T>, ConfigError> {
    env_value(name)
        .map(|value| {
            value
                .trim()
                .parse::<T>()
                .map_err(|_| ConfigError::Invalid(format!("{name} must be a number")))
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use std::{env, fs};

    use super::{DEFAULT_MODEL, embedding_config};

    #[test]
    fn default_model_is_msmarco_minilm_l6_cos() {
        assert_eq!(
            DEFAULT_MODEL,
            "sentence-transformers/msmarco-MiniLM-L6-cos-v5"
        );
    }

    #[test]
    fn environment_overrides_file_configuration() {
        let root = env::temp_dir().join(format!("graphmem-config-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("config.toml"),
            "[embedding]\nmodel = 'file-model'\nrevision = 'file-revision'\nbackend = 'cpu'\n[runtime]\nworker_threads = 2\n",
        )
        .unwrap();
        unsafe { env::set_var("GRAPHMEM_EMBEDDING_MODEL", "environment-model") };
        let config = embedding_config(&root).unwrap();
        unsafe { env::remove_var("GRAPHMEM_EMBEDDING_MODEL") };
        assert_eq!(config.model, "environment-model");
        assert_eq!(config.revision, "file-revision");
        assert_eq!(config.backend, "cpu");
        assert_eq!(config.batch_size, None);
        assert_eq!(super::runtime_config(&root).unwrap().worker_threads, 2);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn batch_size_prefers_environment_and_rejects_zero() {
        let root = env::temp_dir().join(format!("graphmem-batch-config-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("config.toml"), "[embedding]\nbatch_size = 8\n").unwrap();
        assert_eq!(embedding_config(&root).unwrap().batch_size, Some(8));
        unsafe { env::set_var("GRAPHMEM_EMBEDDING_BATCH_SIZE", "32") };
        let from_environment = embedding_config(&root).map(|config| config.batch_size);
        unsafe { env::set_var("GRAPHMEM_EMBEDDING_BATCH_SIZE", "0") };
        let zero = embedding_config(&root);
        unsafe { env::remove_var("GRAPHMEM_EMBEDDING_BATCH_SIZE") };
        assert_eq!(from_environment.unwrap(), Some(32));
        assert!(zero.is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
