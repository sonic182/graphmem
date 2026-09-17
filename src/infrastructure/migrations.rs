use rusqlite::Connection;

use super::sqlite::{Result, now_millis};

const MIGRATIONS: &[(i64, &str)] = &[(1, include_str!("../../migrations/0001_initial.sql"))];

pub fn run(connection: &mut Connection) -> Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
             version INTEGER PRIMARY KEY,
             applied_at INTEGER NOT NULL
         );",
    )?;
    let current = schema_version(connection)?;
    for &(version, sql) in MIGRATIONS {
        if version <= current {
            continue;
        }
        let transaction = connection.transaction()?;
        transaction.execute_batch(sql)?;
        transaction.execute(
            "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
            rusqlite::params![version, now_millis()?],
        )?;
        transaction.commit()?;
    }
    Ok(())
}

pub fn schema_version(connection: &Connection) -> Result<i64> {
    Ok(connection.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
        [],
        |row| row.get(0),
    )?)
}
