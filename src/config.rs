use std::{
    env, fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use thiserror::Error;

const DEFAULT_MODEL: &str = "Qwen/Qwen3-Embedding-0.6B";
const DEFAULT_REVISION: &str = "main";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddingConfig {
    pub enabled: bool,
    pub model: String,
    pub revision: String,
    pub cache_dir: PathBuf,
    pub backend: String,
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
    runtime: Option<FileRuntimeConfig>,
}

#[derive(Debug, Default, Deserialize)]
struct FileEmbeddingConfig {
    enabled: Option<bool>,
    model: Option<String>,
    revision: Option<String>,
    cache_dir: Option<PathBuf>,
    backend: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct FileRuntimeConfig {
    worker_threads: Option<usize>,
}

pub fn embedding_config(data_dir: &Path) -> Result<EmbeddingConfig, ConfigError> {
    let path = data_dir.join("config.toml");
    let file = match fs::read_to_string(path) {
        Ok(contents) => toml::from_str::<FileConfig>(&contents)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => FileConfig::default(),
        Err(error) => return Err(error.into()),
    };
    let embedding = file.embedding.unwrap_or_default();
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
    })
}

pub fn runtime_config(data_dir: &Path) -> Result<RuntimeConfig, ConfigError> {
    let path = data_dir.join("config.toml");
    let file = match fs::read_to_string(path) {
        Ok(contents) => toml::from_str::<FileConfig>(&contents)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => FileConfig::default(),
        Err(error) => return Err(error.into()),
    };
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

#[cfg(test)]
mod tests {
    use std::{env, fs};

    use super::embedding_config;

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
        assert_eq!(super::runtime_config(&root).unwrap().worker_threads, 2);
        fs::remove_dir_all(root).unwrap();
    }
}
