use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Component, Path, PathBuf},
};

use rayon::{ThreadPoolBuilder, prelude::*};

use super::{ApplicationError, Result};
use crate::{
    domain::{
        CodeSymbol, Coverage, Freshness, SymbolChange, SymbolChangeKind, diff_symbols, nest_symbols,
    },
    infrastructure::{
        code_index::{CodeIndex, FileRows, FileStamp, IndexedFile},
        config::code_config,
        outline::{CodeLanguage, Outline, outline},
        repository::{
            ChangedFile, changed_files, file_at, git_repository, list_files, merge_base,
            resolve_commit,
        },
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

    /// How many enclosing symbols the symbol at `index` has.
    pub fn depth(&self, index: usize) -> usize {
        std::iter::successors(self.symbols[index].parent, |&parent| {
            self.symbols[parent].parent
        })
        .count()
    }

    /// The symbols nested at most `max` levels deep, with their outline
    /// index; every symbol when `max` is `None`.
    pub fn within_depth(&self, max: Option<usize>) -> impl Iterator<Item = (usize, &CodeSymbol)> {
        self.symbols
            .iter()
            .enumerate()
            .filter(move |(index, _)| max.is_none_or(|max| self.depth(*index) <= max))
    }
}

pub struct CodeDiff {
    pub base: String,
    pub head: String,
    pub files: Vec<FileDiff>,
    pub skipped: Vec<(String, String)>,
}

impl CodeDiff {
    pub fn to_text(&self) -> String {
        let short = |commit: &str| commit[..commit.len().min(12)].to_owned();
        let words = |text: &str| {
            text.chars()
                .filter(|character| character.is_alphanumeric())
                .collect::<String>()
        };
        let mut lines = vec![format!(
            "merge base {} head {}",
            short(&self.base),
            short(&self.head)
        )];
        for file in &self.files {
            let mut header = format!("{}\t{}", file.path, file.status.as_str());
            if let Some(old_path) = &file.old_path {
                header.push_str(" from ");
                header.push_str(old_path);
            }
            if !matches!(file.coverage, Coverage::Complete) {
                header.push('\t');
                header.push_str(&file.coverage.as_text());
            }
            lines.push(header);
            for change in &file.symbols {
                let symbol = &change.symbol;
                let marker = match change.change {
                    SymbolChangeKind::Added => '+',
                    SymbolChangeKind::Removed => '-',
                    SymbolChangeKind::Modified => '~',
                };
                let mut line = format!(
                    "  {marker} {}-{}\t{} {}",
                    symbol.start.line, symbol.end.line, symbol.kind, symbol.name
                );
                if change.change != SymbolChangeKind::Removed
                    && symbol.signature.contains(['(', ':', '<', '=', ','])
                    && words(&symbol.signature) != words(&symbol.name)
                {
                    line.push('\t');
                    line.push_str(&symbol.signature);
                }
                lines.push(line);
            }
        }
        let mut reasons: Vec<(&str, Vec<&str>)> = Vec::new();
        for (path, reason) in &self.skipped {
            match reasons.iter_mut().find(|(known, _)| known == reason) {
                Some((_, paths)) => paths.push(path),
                None => reasons.push((reason, vec![path])),
            }
        }
        for (reason, paths) in reasons {
            lines.push(format!("skipped ({reason}): {}", paths.join(", ")));
        }
        lines.push(String::new());
        lines.join("\n")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    Added,
    Deleted,
    Modified,
    Renamed,
}

impl FileStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Deleted => "deleted",
            Self::Modified => "modified",
            Self::Renamed => "renamed",
        }
    }
}

pub struct FileDiff {
    pub path: String,
    pub old_path: Option<String>,
    pub status: FileStatus,
    pub coverage: Coverage,
    pub symbols: Vec<SymbolChange>,
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
    match outline_bytes(language, &bytes) {
        Ok((_, outline)) => rows(outline.coverage, outline.symbols),
        Err(reason) => rows(Coverage::Skipped(reason.to_owned()), Vec::new()),
    }
}

fn outline_bytes(
    language: CodeLanguage,
    bytes: &[u8],
) -> std::result::Result<(&str, Outline), &'static str> {
    if bytes[..bytes.len().min(8192)].contains(&0) {
        return Err("binary");
    }
    let source = std::str::from_utf8(bytes).map_err(|_| "not valid UTF-8")?;
    Ok((source, outline(language, source)))
}

pub fn diff(directory: &Path, base: &str, head: &str) -> Result<CodeDiff> {
    let root = checkout_root(directory)?;
    let resolve = |revision: &str| {
        resolve_commit(&root, revision)
            .ok_or_else(|| code_error(format!("{revision}: unknown revision")))
    };
    let head = resolve(head)?;
    let base = merge_base(&root, &resolve(base)?, &head)
        .ok_or_else(|| code_error(format!("{base} and {head} have no common ancestor")))?;
    let changed = changed_files(&root, &base, &head)
        .ok_or_else(|| code_error(format!("git diff failed in {}", root.display())))?;
    let mut report = CodeDiff {
        base,
        head,
        files: Vec::new(),
        skipped: Vec::new(),
    };
    for file in changed {
        match diff_file(&root, &report.base, &report.head, &file) {
            Ok(Some(diff)) => report.files.push(diff),
            Ok(None) => {}
            Err(reason) => report.skipped.push((file.path, reason)),
        }
    }
    Ok(report)
}

fn diff_file(
    root: &Path,
    base: &str,
    head: &str,
    file: &ChangedFile,
) -> std::result::Result<Option<FileDiff>, String> {
    let status = match file.status {
        'A' => FileStatus::Added,
        'D' => FileStatus::Deleted,
        'M' | 'T' => FileStatus::Modified,
        'R' => FileStatus::Renamed,
        _ => return Ok(None),
    };
    let language = CodeLanguage::for_path(Path::new(&file.path))
        .filter(|_| !is_minified(Path::new(&file.path)))
        .ok_or("unsupported file type")?;
    let old_path = file.old_path.as_deref().unwrap_or(&file.path);
    let read = |commit: &str, path: &str| {
        let bytes = file_at(root, commit, path).ok_or("git cat-file failed")?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(format!("larger than {MAX_FILE_BYTES} bytes"));
        }
        Ok(bytes)
    };
    let base_bytes = match status {
        FileStatus::Added => Vec::new(),
        _ => read(base, old_path)?,
    };
    let head_bytes = match status {
        FileStatus::Deleted => Vec::new(),
        _ => read(head, &file.path)?,
    };
    let (base_source, base_outline) = outline_bytes(language, &base_bytes)?;
    let (head_source, head_outline) = outline_bytes(language, &head_bytes)?;
    let coverage = [&base_outline.coverage, &head_outline.coverage]
        .into_iter()
        .find(|coverage| !matches!(coverage, Coverage::Complete))
        .cloned()
        .unwrap_or(Coverage::Complete);
    Ok(Some(FileDiff {
        path: file.path.clone(),
        old_path: file.old_path.clone(),
        status,
        coverage,
        symbols: diff_symbols(
            base_source,
            &definitions(base_outline.symbols),
            head_source,
            &definitions(head_outline.symbols),
        ),
    }))
}

fn definitions(symbols: Vec<CodeSymbol>) -> Vec<CodeSymbol> {
    let usage =
        |symbol: &CodeSymbol| matches!(symbol.kind.as_str(), "component" | "slot" | "expression");
    let inside_definition = symbols
        .iter()
        .map(|symbol| {
            std::iter::successors(symbol.parent, |&parent| symbols[parent].parent)
                .any(|parent| !usage(&symbols[parent]))
        })
        .collect::<Vec<_>>();
    let mut kept = symbols
        .into_iter()
        .zip(inside_definition)
        .filter(|(symbol, inside)| !(usage(symbol) && *inside))
        .map(|(symbol, _)| symbol)
        .collect::<Vec<_>>();
    nest_symbols(&mut kept);
    kept
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
