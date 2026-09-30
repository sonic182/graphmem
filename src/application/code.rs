use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Component, Path, PathBuf},
};

use super::{ApplicationError, Result};
use crate::{
    domain::{CodeSymbol, Coverage, Freshness},
    infrastructure::{
        code_index::{CodeIndex, FileStamp, IndexedFile},
        config::code_config,
        outline::{CodeLanguage, outline},
        repository::{git_repository, list_files},
    },
};

// ponytail: fixed bounds; move them to `[code]` if real repositories hit them.
const MAX_FILES: usize = 20_000;
const MAX_FILE_BYTES: u64 = 1024 * 1024;
pub const MAX_FIND_LIMIT: usize = 100;

pub struct CodeService {
    index: CodeIndex,
}

#[derive(Debug, Default)]
pub struct IndexReport {
    pub root: PathBuf,
    pub indexed: usize,
    pub unchanged: usize,
    pub removed: usize,
    pub skipped: usize,
    pub failed: Vec<String>,
    /// More than `MAX_FILES` source files were found; the rest were ignored.
    pub truncated: bool,
}

pub struct FileOutline {
    pub path: String,
    pub language: String,
    pub coverage: Coverage,
    pub symbols: Vec<CodeSymbol>,
}

pub struct SymbolHit {
    pub path: String,
    pub language: String,
    pub freshness: Freshness,
    pub parent: Option<String>,
    pub symbol: CodeSymbol,
}

enum Refresh {
    Unchanged,
    Indexed,
    Skipped,
    Failed(String),
}

impl CodeService {
    /// Opens the index, or returns `None` when `[code] enabled = false` or
    /// `GRAPHMEM_CODE=off`.
    pub fn open_default() -> Result<Option<Self>> {
        let path = CodeIndex::default_path()?;
        let data_dir = path.parent().unwrap_or(Path::new("."));
        if !code_config(data_dir)?.enabled {
            return Ok(None);
        }
        Ok(Some(Self {
            index: CodeIndex::open(&path)?,
        }))
    }

    /// Indexes the Git checkout containing `directory`. Each linked worktree
    /// is its own checkout, since worktrees can hold different code.
    pub fn index(
        &mut self,
        directory: &Path,
        mut progress: impl FnMut(usize, usize),
    ) -> Result<IndexReport> {
        let root = checkout_root(directory)?;
        let checkout = self.index.ensure_checkout(&root_key(&root)?)?;
        let mut paths = list_files(&root)
            .ok_or_else(|| code_error(format!("git ls-files failed in {}", root.display())))?;
        paths.retain(|path| CodeLanguage::for_path(path).is_some() && !is_minified(path));
        let mut report = IndexReport {
            root: root.clone(),
            truncated: paths.len() > MAX_FILES,
            ..IndexReport::default()
        };
        paths.truncate(MAX_FILES);

        let known = self
            .index
            .files(checkout)?
            .into_iter()
            .map(|file| (file.path.clone(), file))
            .collect::<HashMap<_, _>>();
        let mut kept = HashSet::new();
        for (done, path) in paths.iter().enumerate() {
            progress(done, paths.len());
            let Some(path) = path.to_str() else {
                report.skipped += 1;
                continue;
            };
            match self.refresh(checkout, &root, path, known.get(path))? {
                Refresh::Unchanged => report.unchanged += 1,
                Refresh::Indexed => report.indexed += 1,
                Refresh::Skipped => {
                    report.skipped += 1;
                    continue;
                }
                Refresh::Failed(error) => {
                    report.failed.push(format!("{path}: {error}"));
                    continue;
                }
            }
            kept.insert(path);
        }
        progress(paths.len(), paths.len());
        for path in known.keys() {
            if !kept.contains(path.as_str()) {
                self.index.remove_file(checkout, path)?;
                report.removed += 1;
            }
        }
        Ok(report)
    }

    /// Outlines one file of the checkout containing `directory`, re-indexing
    /// it first when it changed, so the result always matches the file.
    pub fn outline(&mut self, directory: &Path, path: &str) -> Result<FileOutline> {
        let root = checkout_root(directory)?;
        let path = checkout_path(&root, path)?;
        if CodeLanguage::for_path(Path::new(&path)).is_none() {
            return Err(code_error(format!("{path}: unsupported file type")));
        }
        let checkout = self.index.ensure_checkout(&root_key(&root)?)?;
        let known = self.index.file(checkout, &path)?;
        match self.refresh(checkout, &root, &path, known.as_ref())? {
            Refresh::Unchanged | Refresh::Indexed => {}
            Refresh::Skipped => {
                self.index.remove_file(checkout, &path)?;
                return Err(code_error(format!(
                    "{path}: not indexed (binary, larger than {MAX_FILE_BYTES} bytes, or not a regular file)"
                )));
            }
            Refresh::Failed(error) => return Err(code_error(format!("{path}: {error}"))),
        }
        let file = self
            .index
            .file(checkout, &path)?
            .ok_or(ApplicationError::NotFound("indexed file"))?;
        Ok(FileOutline {
            symbols: self.index.symbols(file.id)?,
            path: file.path,
            language: file.language,
            coverage: file.coverage,
        })
    }

    /// Symbols whose name matches `query`, exact names first. Several matches
    /// are returned as they are; none is picked as "the" definition.
    pub fn find_symbol(
        &self,
        directory: &Path,
        query: &str,
        kind: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SymbolHit>> {
        let query = query.trim();
        if query.is_empty() {
            return Err(code_error("query must not be empty"));
        }
        let root = checkout_root(directory)?;
        let Some(checkout) = self.index.checkout(&root_key(&root)?)? else {
            return Err(code_error(format!(
                "{} is not indexed; run `gmem code index` in it",
                root.display()
            )));
        };
        let matches = self
            .index
            .find(checkout, query, kind, limit.clamp(1, MAX_FIND_LIMIT))?;
        Ok(matches
            .into_iter()
            .map(|found| SymbolHit {
                freshness: match fs::symlink_metadata(root.join(&found.path)) {
                    Ok(metadata) if FileStamp::of(&metadata) == found.stamp => Freshness::Fresh,
                    Ok(_) => Freshness::Stale,
                    Err(_) => Freshness::Missing,
                },
                path: found.path,
                language: found.language,
                parent: found.parent,
                symbol: found.symbol,
            })
            .collect())
    }

    fn refresh(
        &mut self,
        checkout: i64,
        root: &Path,
        path: &str,
        known: Option<&IndexedFile>,
    ) -> Result<Refresh> {
        let absolute = root.join(path);
        let Ok(metadata) = fs::symlink_metadata(&absolute) else {
            return Ok(Refresh::Skipped);
        };
        if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
            return Ok(Refresh::Skipped);
        }
        let stamp = FileStamp::of(&metadata);
        if known.is_some_and(|file| file.stamp == stamp) {
            return Ok(Refresh::Unchanged);
        }
        let Some(language) = CodeLanguage::for_path(&absolute) else {
            return Ok(Refresh::Skipped);
        };
        let bytes = match fs::read(&absolute) {
            Ok(bytes) => bytes,
            Err(error) => return Ok(Refresh::Failed(error.to_string())),
        };
        if bytes[..bytes.len().min(8192)].contains(&0) {
            return Ok(Refresh::Skipped);
        }
        let Ok(source) = String::from_utf8(bytes) else {
            return Ok(Refresh::Failed("not valid UTF-8".to_owned()));
        };
        let outline = outline(language, &source);
        self.index.replace_file(
            checkout,
            path,
            language.name(),
            stamp,
            &outline.coverage,
            &outline.symbols,
        )?;
        Ok(Refresh::Indexed)
    }
}

fn checkout_root(directory: &Path) -> Result<PathBuf> {
    git_repository(directory)
        .and_then(|repository| repository.checkout_root)
        .ok_or_else(|| {
            code_error(format!(
                "{} is not inside a Git checkout",
                directory.display()
            ))
        })
}

fn root_key(root: &Path) -> Result<String> {
    root.to_str()
        .map(str::to_owned)
        .ok_or_else(|| code_error(format!("{} is not valid UTF-8", root.display())))
}

/// Resolves a checkout-relative (or absolute) path to its checkout-relative
/// form, refusing `..`, symlinks, and anything that resolves outside `root`.
fn checkout_path(root: &Path, path: &str) -> Result<String> {
    let requested = Path::new(path);
    if requested
        .components()
        .any(|component| component == Component::ParentDir)
    {
        return Err(code_error(format!("{path}: `..` is not allowed")));
    }
    let joined = root.join(requested);
    let metadata = fs::symlink_metadata(&joined)
        .map_err(|_| code_error(format!("{path}: no such file in {}", root.display())))?;
    if metadata.file_type().is_symlink() {
        return Err(code_error(format!("{path}: symlinks are not followed")));
    }
    let resolved = joined
        .canonicalize()
        .map_err(|error| code_error(error.to_string()))?;
    let relative = resolved
        .strip_prefix(root)
        .map_err(|_| code_error(format!("{path}: outside the checkout {}", root.display())))?;
    root_key(relative)
}

fn is_minified(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".min.js"))
}

fn code_error(message: impl Into<String>) -> ApplicationError {
    ApplicationError::Code(message.into())
}
