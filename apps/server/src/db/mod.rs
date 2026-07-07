//! Persistent auth state in a local libsql (SQLite) database shared by the
//! daemon and the `railyard-server user` commands. Each table's queries and
//! types live in its own module; this one owns the connection and schema.

mod invite;
mod user;

pub(crate) use invite::token_hash;

use libsql::{Builder, Connection, Value};
use std::io;
use std::time::Duration;

use crate::paths;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    public_key TEXT,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS invites (
    token_hash TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    expires_at INTEGER NOT NULL,
    redeemed_at INTEGER,
    created_at INTEGER NOT NULL
);
";

pub(crate) struct Db {
    conn: Connection,
}

impl Db {
    /// Opens the auth database at its well-known path. The daemon and the
    /// CLI open it concurrently; WAL mode plus a busy timeout make that safe.
    pub(crate) async fn open() -> io::Result<Self> {
        let path = paths::database_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let path = path
            .to_str()
            .ok_or_else(|| io::Error::other("database path is not valid UTF-8"))?;
        let db = Builder::new_local(path).build().await.map_err(db_error)?;
        let conn = db.connect().map_err(db_error)?;

        conn.busy_timeout(Duration::from_secs(5)).map_err(db_error)?;
        conn.query("PRAGMA journal_mode = WAL", ())
            .await
            .map_err(db_error)?;
        conn.execute_batch(SCHEMA).await.map_err(db_error)?;

        Ok(Self { conn })
    }
}

fn text_column(row: &libsql::Row, index: i32) -> io::Result<String> {
    match row.get_value(index).map_err(db_error)? {
        Value::Text(value) => Ok(value),
        other => Err(io::Error::other(format!(
            "expected text in column {index}, got {other:?}"
        ))),
    }
}

fn integer_column(row: &libsql::Row, index: i32) -> io::Result<i64> {
    match row.get_value(index).map_err(db_error)? {
        Value::Integer(value) => Ok(value),
        other => Err(io::Error::other(format!(
            "expected integer in column {index}, got {other:?}"
        ))),
    }
}

fn db_error(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(format!("auth database error: {error}"))
}
