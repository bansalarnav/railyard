use libsql::{Value, params};
use std::io;

use super::{Db, db_error, integer_column, text_column};

pub(crate) struct User {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) has_key: bool,
    pub(crate) created_at: u64,
}

impl Db {
    pub(crate) async fn create_user(&self, name: &str, now: u64) -> io::Result<String> {
        if self.user_id_by_name(name).await?.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("user {name} already exists"),
            ));
        }

        let user_id = new_user_id();
        self.conn
            .execute(
                "INSERT INTO users (id, name, created_at) VALUES (?1, ?2, ?3)",
                (user_id.as_str(), name, now as i64),
            )
            .await
            .map_err(db_error)?;

        Ok(user_id)
    }

    pub(crate) async fn list_users(&self) -> io::Result<Vec<User>> {
        let mut rows = self
            .conn
            .query(
                "SELECT id, name, public_key, created_at FROM users ORDER BY created_at, name",
                (),
            )
            .await
            .map_err(db_error)?;

        let mut users = Vec::new();
        while let Some(row) = rows.next().await.map_err(db_error)? {
            users.push(User {
                id: text_column(&row, 0)?,
                name: text_column(&row, 1)?,
                has_key: !matches!(row.get_value(2).map_err(db_error)?, Value::Null),
                created_at: integer_column(&row, 3)? as u64,
            });
        }

        Ok(users)
    }
    pub(crate) async fn remove_user(&self, name: &str) -> io::Result<bool> {
        let Some(user_id) = self.user_id_by_name(name).await? else {
            return Ok(false);
        };

        self.conn
            .execute(
                "DELETE FROM invites WHERE user_id = ?1",
                params![user_id.as_str()],
            )
            .await
            .map_err(db_error)?;
        self.conn
            .execute("DELETE FROM users WHERE id = ?1", params![user_id.as_str()])
            .await
            .map_err(db_error)?;

        Ok(true)
    }
    pub(crate) async fn public_key_for(&self, key_id: &str) -> io::Result<Option<String>> {
        let mut rows = self
            .conn
            .query(
                "SELECT public_key FROM users WHERE id = ?1 AND public_key IS NOT NULL",
                params![key_id],
            )
            .await
            .map_err(db_error)?;

        match rows.next().await.map_err(db_error)? {
            Some(row) => Ok(Some(text_column(&row, 0)?)),
            None => Ok(None),
        }
    }

    async fn user_id_by_name(&self, name: &str) -> io::Result<Option<String>> {
        let mut rows = self
            .conn
            .query("SELECT id FROM users WHERE name = ?1", params![name])
            .await
            .map_err(db_error)?;

        match rows.next().await.map_err(db_error)? {
            Some(row) => Ok(Some(text_column(&row, 0)?)),
            None => Ok(None),
        }
    }
}

fn new_user_id() -> String {
    use rand::RngCore;

    let mut bytes = [0u8; 8];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    format!("usr_{}", hex::encode(bytes))
}
