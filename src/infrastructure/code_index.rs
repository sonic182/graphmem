use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};

use super::sqlite::{Database, Result, StorageError};
use crate::domain::{CodeSymbol, Coverage, SourcePoint};

/// Bump when the schema or the extracted symbols change; a mismatch rebuilds
/// the index from scratch, since it can always be regenerated from source.
const INDEX_VERSION: i64 = 1;

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

    pub fn checkout(&self, root: &str) -> Result<Option<i64>> {
        Ok(self
            .connection
            .query_row("SELECT id FROM checkouts WHERE root = ?1", [root], |row| {
                row.get(0)
            })
            .optional()?)
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

    /// Replaces one file's symbols atomically, so a reader never sees a file
    /// half-indexed.
    pub fn replace_file(
        &mut self,
        checkout: i64,
        path: &str,
        language: &str,
        stamp: FileStamp,
        coverage: &Coverage,
        symbols: &[CodeSymbol],
    ) -> Result<()> {
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "DELETE FROM files WHERE checkout_id = ?1 AND path = ?2",
            params![checkout, path],
        )?;
        transaction.execute(
            "INSERT INTO files (checkout_id, path, language, size, modified_ns, coverage)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                checkout,
                path,
                language,
                stamp.size,
                stamp.modified_ns,
                coverage.as_text()
            ],
        )?;
        let file = transaction.last_insert_rowid();
        {
            let mut insert = transaction.prepare(
                "INSERT INTO symbols (file_id, ordinal, parent_ordinal, name, kind,
                     start_line, start_column, end_line, end_column, signature)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            )?;
            for (ordinal, symbol) in symbols.iter().enumerate() {
                insert.execute(params![
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
        transaction.commit()?;
        Ok(())
    }

    pub fn remove_file(&self, checkout: i64, path: &str) -> Result<()> {
        self.connection.execute(
            "DELETE FROM files WHERE checkout_id = ?1 AND path = ?2",
            params![checkout, path],
        )?;
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
    /// prefix matches; both ignore ASCII case.
    pub fn find(
        &self,
        checkout: i64,
        query: &str,
        kind: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SymbolMatch>> {
        let escaped = query
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        let mut statement = self.connection.prepare(
            "SELECT s.name, s.kind, s.parent_ordinal, s.start_line, s.start_column,
                    s.end_line, s.end_column, s.signature,
                    f.path, f.language, f.size, f.modified_ns, p.name
             FROM symbols s
             JOIN files f ON f.id = s.file_id
             LEFT JOIN symbols p ON p.file_id = s.file_id AND p.ordinal = s.parent_ordinal
             WHERE f.checkout_id = ?1
               AND s.name LIKE ?2 ESCAPE '\\'
               AND (?5 IS NULL OR s.kind = ?5)
             ORDER BY
               CASE WHEN s.name = ?3 OR s.name LIKE ?4 ESCAPE '\\' THEN 0 ELSE 1 END,
               length(s.name), f.path, s.start_line
             LIMIT ?6",
        )?;
        let matches = statement
            .query_map(
                params![
                    checkout,
                    format!("{escaped}%"),
                    query,
                    format!("{escaped}/%"),
                    kind,
                    limit as i64
                ],
                |row| {
                    Ok(SymbolMatch {
                        symbol: symbol(row)?,
                        path: row.get(8)?,
                        language: row.get(9)?,
                        stamp: FileStamp {
                            size: row.get(10)?,
                            modified_ns: row.get(11)?,
                        },
                        parent: row.get(12)?,
                    })
                },
            )?
            .collect::<rusqlite::Result<_>>()?;
        Ok(matches)
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
