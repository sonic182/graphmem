use std::path::{Path, PathBuf};

use rusqlite::{Connection, ErrorCode, OptionalExtension, params};

use super::sqlite::{Database, Result, StorageError};
use crate::domain::{CodeSymbol, Coverage, SourcePoint};

/// Bump when the schema or the extracted symbols change; a mismatch rebuilds
/// the index from scratch, since it can always be regenerated from source.
const INDEX_VERSION: i64 = 3;

const USAGE_KINDS: &str = "'import', 'component', 'slot', 'expression'";

const SCHEMA: &str = "
    DROP TABLE IF EXISTS symbols;
    DROP TABLE IF EXISTS files;
    DROP TABLE IF EXISTS checkouts;
    CREATE TABLE checkouts (
        id INTEGER PRIMARY KEY,
        root TEXT NOT NULL UNIQUE
    );
    CREATE TABLE files (
        id INTEGER PRIMARY KEY,
        checkout_id INTEGER NOT NULL REFERENCES checkouts(id) ON DELETE CASCADE,
        path TEXT NOT NULL,
        language TEXT NOT NULL,
        size INTEGER NOT NULL,
        modified_ns INTEGER NOT NULL,
        coverage TEXT NOT NULL,
        UNIQUE (checkout_id, path)
    );
    CREATE TABLE symbols (
        file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
        ordinal INTEGER NOT NULL,
        parent_ordinal INTEGER,
        name TEXT NOT NULL COLLATE NOCASE,
        kind TEXT NOT NULL,
        start_line INTEGER NOT NULL,
        start_column INTEGER NOT NULL,
        end_line INTEGER NOT NULL,
        end_column INTEGER NOT NULL,
        signature TEXT NOT NULL,
        PRIMARY KEY (file_id, ordinal)
    );
    CREATE INDEX symbols_name ON symbols(name);
";

/// Size and modification time; a mismatch means the file changed since it was
/// indexed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStamp {
    pub size: i64,
    pub modified_ns: i64,
}

impl FileStamp {
    pub fn of(metadata: &std::fs::Metadata) -> Self {
        let modified_ns = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |duration| duration.as_nanos() as i64);
        Self {
            size: metadata.len() as i64,
            modified_ns,
        }
    }
}

#[derive(Debug, Clone)]
pub struct IndexedFile {
    pub id: i64,
    pub path: String,
    pub language: String,
    pub stamp: FileStamp,
    pub coverage: Coverage,
}

#[derive(Debug, Clone)]
pub struct FileRows {
    pub path: String,
    pub language: String,
    pub stamp: FileStamp,
    pub coverage: Coverage,
    pub symbols: Vec<CodeSymbol>,
}

#[derive(Debug, Clone)]
pub struct SymbolMatch {
    pub path: String,
    pub language: String,
    pub stamp: FileStamp,
    pub parent: Option<String>,
    pub symbol: CodeSymbol,
}

pub struct CodeIndex {
    connection: Connection,
}

impl CodeIndex {
    pub fn default_path() -> Result<PathBuf> {
        let database = Database::default_path()?;
        let data_dir = database.parent().ok_or(StorageError::Invalid {
            field: "database path",
            message: "default database has no parent directory",
        })?;
        Ok(data_dir.join("code.sqlite"))
    }

    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        match Self::connect(path) {
            Err(StorageError::Sqlite(rusqlite::Error::SqliteFailure(error, _)))
                if matches!(
                    error.code,
                    ErrorCode::NotADatabase | ErrorCode::DatabaseCorrupt
                ) =>
            {
                for suffix in ["", "-wal", "-shm"] {
                    let mut file = path.as_os_str().to_owned();
                    file.push(suffix);
                    match std::fs::remove_file(&file) {
                        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                            return Err(error.into());
                        }
                        _ => {}
                    }
                }
                Self::connect(path)
            }
            result => result,
        }
    }

    fn connect(path: &Path) -> Result<Self> {
        let connection = Connection::open(path)?;
        connection.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             PRAGMA busy_timeout = 5000;",
        )?;
        let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version != INDEX_VERSION {
            connection.execute_batch(&format!(
                "BEGIN; {SCHEMA} PRAGMA user_version = {INDEX_VERSION}; COMMIT;"
            ))?;
        }
        Ok(Self { connection })
    }

    pub fn ensure_checkout(&self, root: &str) -> Result<i64> {
        self.connection.execute(
            "INSERT INTO checkouts (root) VALUES (?1) ON CONFLICT (root) DO NOTHING",
            [root],
        )?;
        Ok(self.connection.query_row(
            "SELECT id FROM checkouts WHERE root = ?1",
            [root],
            |row| row.get(0),
        )?)
    }

    pub fn files(&self, checkout: i64) -> Result<Vec<IndexedFile>> {
        let mut statement = self.connection.prepare(
            "SELECT id, path, language, size, modified_ns, coverage
             FROM files WHERE checkout_id = ?1 ORDER BY path",
        )?;
        let files = statement
            .query_map([checkout], indexed_file)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(files)
    }

    pub fn file(&self, checkout: i64, path: &str) -> Result<Option<IndexedFile>> {
        Ok(self
            .connection
            .query_row(
                "SELECT id, path, language, size, modified_ns, coverage
                 FROM files WHERE checkout_id = ?1 AND path = ?2",
                params![checkout, path],
                indexed_file,
            )
            .optional()?)
    }

    /// Replaces the symbols of several files in one transaction, so a reader
    /// never sees a file half-indexed.
    pub fn replace_files(&mut self, checkout: i64, files: &[FileRows]) -> Result<()> {
        let transaction = self.connection.transaction()?;
        {
            let mut delete =
                transaction.prepare("DELETE FROM files WHERE checkout_id = ?1 AND path = ?2")?;
            let mut insert_file = transaction.prepare(
                "INSERT INTO files (checkout_id, path, language, size, modified_ns, coverage)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?;
            let mut insert_symbol = transaction.prepare(
                "INSERT INTO symbols (file_id, ordinal, parent_ordinal, name, kind,
                     start_line, start_column, end_line, end_column, signature)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            )?;
            for rows in files {
                delete.execute(params![checkout, rows.path])?;
                let file = insert_file.insert(params![
                    checkout,
                    rows.path,
                    rows.language,
                    rows.stamp.size,
                    rows.stamp.modified_ns,
                    rows.coverage.as_text()
                ])?;
                for (ordinal, symbol) in rows.symbols.iter().enumerate() {
                    insert_symbol.execute(params![
                        file,
                        ordinal as i64,
                        symbol.parent.map(|parent| parent as i64),
                        symbol.name,
                        symbol.kind,
                        symbol.start.line as i64,
                        symbol.start.column as i64,
                        symbol.end.line as i64,
                        symbol.end.column as i64,
                        symbol.signature,
                    ])?;
                }
            }
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn remove_files(&mut self, checkout: i64, paths: &[&str]) -> Result<()> {
        let transaction = self.connection.transaction()?;
        {
            let mut delete =
                transaction.prepare("DELETE FROM files WHERE checkout_id = ?1 AND path = ?2")?;
            for path in paths {
                delete.execute(params![checkout, path])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn symbols(&self, file: i64) -> Result<Vec<CodeSymbol>> {
        let mut statement = self.connection.prepare(
            "SELECT name, kind, parent_ordinal, start_line, start_column, end_line, end_column,
                    signature
             FROM symbols WHERE file_id = ?1 ORDER BY ordinal",
        )?;
        let symbols = statement
            .query_map([file], symbol)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(symbols)
    }

    /// Exact names first (for Elixir, `name` also matches `name/arity`), then
    /// names ending in `.query` (`ConsentLive` finds `App.ConsentLive`), then
    /// prefix matches; all ignore ASCII case. Returns up to `limit` matches
    /// and the total number of matches.
    pub fn find(
        &self,
        checkout: i64,
        query: &str,
        kind: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<SymbolMatch>, usize)> {
        let escaped = query
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let mut statement = self.connection.prepare(&format!(
            "SELECT s.name, s.kind, s.parent_ordinal, s.start_line, s.start_column,
                    s.end_line, s.end_column, s.signature,
                    f.path, f.language, f.size, f.modified_ns, p.name, COUNT(*) OVER ()
             FROM symbols s
             JOIN files f ON f.id = s.file_id
             LEFT JOIN symbols p ON p.file_id = s.file_id AND p.ordinal = s.parent_ordinal
             WHERE f.checkout_id = ?1
               AND (s.name LIKE ?2 ESCAPE '\\' OR s.name LIKE ?7 ESCAPE '\\')
               AND ((?5 IS NULL AND s.kind NOT IN ({USAGE_KINDS})) OR s.kind = ?5)
             ORDER BY
               CASE WHEN s.name = ?3 OR s.name LIKE ?4 ESCAPE '\\' THEN 0
                    WHEN s.name LIKE ?7 ESCAPE '\\' THEN 1
                    ELSE 2 END,
               length(s.name), f.path, s.start_line
             LIMIT ?6"
        ))?;
        let mut total = 0;
        let matches = statement
            .query_map(
                params![
                    checkout,
                    format!("{escaped}%"),
                    query,
                    format!("{escaped}/%"),
                    kind,
                    limit as i64,
                    format!("%.{escaped}"),
                ],
                |row| {
                    Ok((
                        SymbolMatch {
                            symbol: symbol(row)?,
                            path: row.get(8)?,
                            language: row.get(9)?,
                            stamp: FileStamp {
                                size: row.get(10)?,
                                modified_ns: row.get(11)?,
                            },
                            parent: row.get(12)?,
                        },
                        row.get::<_, i64>(13)? as usize,
                    ))
                },
            )?
            .map(|found| {
                found.map(|(found, count)| {
                    total = count;
                    found
                })
            })
            .collect::<rusqlite::Result<_>>()?;
        Ok((matches, total))
    }
}

fn indexed_file(row: &rusqlite::Row<'_>) -> rusqlite::Result<IndexedFile> {
    Ok(IndexedFile {
        id: row.get(0)?,
        path: row.get(1)?,
        language: row.get(2)?,
        stamp: FileStamp {
            size: row.get(3)?,
            modified_ns: row.get(4)?,
        },
        coverage: Coverage::from_text(&row.get::<_, String>(5)?),
    })
}

/// Reads the eight leading symbol columns in `symbols()` order.
fn symbol(row: &rusqlite::Row<'_>) -> rusqlite::Result<CodeSymbol> {
    let position = |line: usize, column: usize| -> rusqlite::Result<SourcePoint> {
        Ok(SourcePoint {
            line: row.get::<_, i64>(line)? as usize,
            column: row.get::<_, i64>(column)? as usize,
        })
    };
    Ok(CodeSymbol {
        name: row.get(0)?,
        kind: row.get(1)?,
        parent: row.get::<_, Option<i64>>(2)?.map(|parent| parent as usize),
        start: position(3, 4)?,
        end: position(5, 6)?,
        signature: row.get(7)?,
    })
}
