use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Component, Path, PathBuf},
};

use rayon::{ThreadPoolBuilder, prelude::*};

use super::{ApplicationError, Result};
use crate::{
    domain::{CodeSymbol, Coverage, Freshness},
    infrastructure::{
        code_index::{CodeIndex, FileRows, FileStamp, IndexedFile},
        config::code_config,
        outline::{CodeLanguage, outline},
        repository::{git_repository, list_files},
    },
};

const MAX_FILE_BYTES: u64 = 1024 * 1024;
const WRITE_BATCH: usize = 256;
pub const MAX_FIND_LIMIT: usize = 100;

pub struct CodeService {
    index: CodeIndex,
    max_files: usize,
    index_threads: Option<usize>,
}

#[derive(Debug, Default)]
pub struct IndexReport {
    pub root: PathBuf,
    pub indexed: usize,
    pub unchanged: usize,
    pub removed: usize,
    pub skipped: usize,
    pub failed: Vec<String>,
    /// More than `[code] max_files` source files were found; the rest were
    /// ignored.
    pub truncated: bool,
}

pub struct FoundSymbols {
    pub hits: Vec<SymbolHit>,
    pub total: usize,
    pub truncated: bool,
}

pub struct FileOutline {
    pub path: String,
    pub language: String,
    pub coverage: Coverage,
    pub symbols: Vec<CodeSymbol>,
}

impl FileOutline {
    /// The declared imports, in source order.
    pub fn imports(&self) -> impl Iterator<Item = &CodeSymbol> {
        self.symbols.iter().filter(|symbol| symbol.kind == "import")
    }
}

pub struct SymbolHit {
    pub path: String,
    pub language: String,
    pub freshness: Freshness,
    pub parent: Option<String>,
    pub symbol: CodeSymbol,
}

enum Stat {
    Unchanged,
    Changed(FileStamp),
    Skipped,
}

enum Parsed {
    Rows(FileRows),
    Failed(String),
}

impl CodeService {
    /// Opens the index, or returns `None` when `[code] enabled = false` or
    /// `GRAPHMEM_CODE=off`.
    pub fn open_default() -> Result<Option<Self>> {
        let path = CodeIndex::default_path()?;
        let data_dir = path.parent().unwrap_or(Path::new("."));
        let config = code_config(data_dir)?;
        if !config.enabled {
            return Ok(None);
        }
        Ok(Some(Self {
            index: CodeIndex::open(&path)?,
            max_files: config.max_files,
            index_threads: config.index_threads,
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
            truncated: paths.len() > self.max_files,
            ..IndexReport::default()
        };
        paths.truncate(self.max_files);

        let known = self
            .index
            .files(checkout)?
            .into_iter()
            .map(|file| (file.path.clone(), file))
            .collect::<HashMap<_, _>>();
        let mut kept = HashSet::new();
        let mut pending = Vec::new();
        for path in &paths {
            let Some(path) = path.to_str() else {
                report.skipped += 1;
                continue;
            };
            match stat(&root, path, known.get(path)) {
                Stat::Unchanged => {
                    if known
                        .get(path)
                        .is_some_and(|file| matches!(file.coverage, Coverage::Skipped(_)))
                    {
                        report.skipped += 1;
                    } else {
                        report.unchanged += 1;
                    }
                    kept.insert(path);
                }
                Stat::Changed(stamp) => pending.push((path, stamp)),
                Stat::Skipped => report.skipped += 1,
            }
        }
        if !pending.is_empty() {
            let pool = ThreadPoolBuilder::new()
                .num_threads(self.thread_count())
                .build()
                .map_err(|error| code_error(format!("indexing threads: {error}")))?;
            for (batch, chunk) in pending.chunks(WRITE_BATCH).enumerate() {
                progress(batch * WRITE_BATCH, pending.len());
                let parsed = pool.install(|| {
                    chunk
                        .par_iter()
                        .map(|(path, stamp)| parse_file(&root, path, *stamp))
                        .collect::<Vec<_>>()
                });
                let mut rows = Vec::with_capacity(chunk.len());
                for ((path, _), parsed) in chunk.iter().zip(parsed) {
                    match parsed {
                        Parsed::Rows(file) => {
                            if matches!(file.coverage, Coverage::Skipped(_)) {
                                report.skipped += 1;
                            } else {
                                report.indexed += 1;
                            }
                            rows.push(file);
                            kept.insert(path);
                        }
                        Parsed::Failed(error) => report.failed.push(format!("{path}: {error}")),
                    }
                }
                self.index.replace_files(checkout, &rows)?;
            }
        }
        progress(pending.len(), pending.len());
        let removed = known
            .keys()
            .map(String::as_str)
            .filter(|path| !kept.contains(path))
            .collect::<Vec<_>>();
        self.index.remove_files(checkout, &removed)?;
        report.removed = removed.len();
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
        match stat(&root, &path, known.as_ref()) {
            Stat::Unchanged => {}
            Stat::Changed(stamp) => match parse_file(&root, &path, stamp) {
                Parsed::Rows(file) => self.index.replace_files(checkout, &[file])?,
                Parsed::Failed(error) => return Err(code_error(format!("{path}: {error}"))),
            },
            Stat::Skipped => {
                self.index.remove_files(checkout, &[&path])?;
                return Err(code_error(format!(
                    "{path}: not indexed (larger than {MAX_FILE_BYTES} bytes, not a regular \
                     file, or outside the checkout)"
                )));
            }
        }
        let file = self
            .index
            .file(checkout, &path)?
            .ok_or(ApplicationError::NotFound("indexed file"))?;
        if let Coverage::Skipped(reason) = &file.coverage {
            return Err(code_error(format!("{path}: not indexed ({reason})")));
        }
        Ok(FileOutline {
            symbols: self.index.symbols(file.id)?,
            path: file.path,
            language: file.language,
            coverage: file.coverage,
        })
    }

    /// Symbols whose name matches `query`, exact names first, after refreshing
    /// the checkout's index so no file is missed. Several matches are returned
    /// as they are; none is picked as "the" definition.
    pub fn find_symbol(
        &mut self,
        directory: &Path,
        query: &str,
        kind: Option<&str>,
        limit: usize,
    ) -> Result<FoundSymbols> {
        let query = query.trim();
        if query.is_empty() {
            return Err(code_error("query must not be empty"));
        }
        let report = self.index(directory, |_, _| {})?;
        let root = report.root;
        let checkout = self.index.ensure_checkout(&root_key(&root)?)?;
        let (matches, total) =
            self.index
                .find(checkout, query, kind, limit.clamp(1, MAX_FIND_LIMIT))?;
        let hits = matches
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
            .collect();
        Ok(FoundSymbols {
            hits,
            total,
            truncated: report.truncated,
        })
    }

    fn thread_count(&self) -> usize {
        self.index_threads.unwrap_or_else(|| {
            std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get)
        })
    }
}

fn stat(root: &Path, path: &str, known: Option<&IndexedFile>) -> Stat {
    let Ok(metadata) = fs::symlink_metadata(root.join(path)) else {
        return Stat::Skipped;
    };
    if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
        return Stat::Skipped;
    }
    let stamp = FileStamp::of(&metadata);
    if known.is_some_and(|file| file.stamp == stamp) {
        return Stat::Unchanged;
    }
    match root.join(path).canonicalize() {
        Ok(real) if real.starts_with(root) => Stat::Changed(stamp),
        _ => Stat::Skipped,
    }
}

fn parse_file(root: &Path, path: &str, stamp: FileStamp) -> Parsed {
    let absolute = root.join(path);
    let Some(language) = CodeLanguage::for_path(&absolute) else {
        return Parsed::Failed("unsupported file type".to_owned());
    };
    let bytes = match fs::read(&absolute) {
        Ok(bytes) => bytes,
        Err(error) => return Parsed::Failed(error.to_string()),
    };
    let rows = |coverage, symbols| {
        Parsed::Rows(FileRows {
            path: path.to_owned(),
            language: language.name().to_owned(),
            stamp,
            coverage,
            symbols,
        })
    };
    if bytes[..bytes.len().min(8192)].contains(&0) {
        return rows(Coverage::Skipped("binary".to_owned()), Vec::new());
    }
    let Ok(source) = String::from_utf8(bytes) else {
        return rows(Coverage::Skipped("not valid UTF-8".to_owned()), Vec::new());
    };
    let outline = outline(language, &source);
    rows(outline.coverage, outline.symbols)
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
        .is_some_and(|name| name.ends_with(".min.js") || name.ends_with(".min.css"))
}

fn code_error(message: impl Into<String>) -> ApplicationError {
    ApplicationError::Code(message.into())
}
