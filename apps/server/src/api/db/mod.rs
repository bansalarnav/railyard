mod invite;
mod project;
mod release;
mod user;

pub(crate) use invite::token_hash;
pub(crate) use project::Project;
pub(crate) use release::Release;
pub(crate) use user::AuthUser;

use anyhow::{Result, anyhow, bail};
use libsql::{Builder, Connection, Value};
use std::time::Duration;

use crate::paths;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    public_key TEXT,
    project_id TEXT,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS invites (
    token_hash TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    expires_at INTEGER NOT NULL,
    redeemed_at INTEGER,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS projects (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS releases (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    status TEXT NOT NULL,
    message TEXT,
    error TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
";

pub(crate) struct Db {
    conn: Connection,
}

impl Db {
    pub(crate) async fn open() -> Result<Self> {
        let path = paths::database_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let path = path
            .to_str()
            .ok_or_else(|| anyhow!("database path is not valid UTF-8"))?;
        let db = Builder::new_local(path).build().await.map_err(db_error)?;
        let conn = db.connect().map_err(db_error)?;

        conn.busy_timeout(Duration::from_secs(5))
            .map_err(db_error)?;
        conn.query("PRAGMA journal_mode = WAL", ())
            .await
            .map_err(db_error)?;

        conn.execute_batch(SCHEMA).await.map_err(db_error)?;

        Ok(Self { conn })
    }
}

fn text_column(row: &libsql::Row, index: i32) -> Result<String> {
    match row.get_value(index).map_err(db_error)? {
        Value::Text(value) => Ok(value),
        other => bail!("expected text in column {index}, got {other:?}"),
    }
}

fn optional_text_column(row: &libsql::Row, index: i32) -> Result<Option<String>> {
    match row.get_value(index).map_err(db_error)? {
        Value::Text(value) => Ok(Some(value)),
        Value::Null => Ok(None),
        other => bail!("expected text or null in column {index}, got {other:?}"),
    }
}

fn integer_column(row: &libsql::Row, index: i32) -> Result<i64> {
    match row.get_value(index).map_err(db_error)? {
        Value::Integer(value) => Ok(value),
        other => bail!("expected integer in column {index}, got {other:?}"),
    }
}

fn db_error(error: impl std::fmt::Display) -> anyhow::Error {
    anyhow!("database error: {error}")
}
