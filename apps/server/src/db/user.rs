use libsql::{Value, params};
use std::io;

use super::{Db, db_error, integer_column, optional_text_column, text_column};

pub(crate) struct User {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) project_id: Option<String>,
    pub(crate) has_key: bool,
    pub(crate) created_at: u64,
}

/// The user a verified request key belongs to; the auth middleware attaches
/// this to the request so handlers can authorize against it. `project_id` of
/// `None` means a server-wide admin.
#[derive(Clone)]
pub(crate) struct AuthUser {
    pub(crate) id: String,
    pub(crate) project_id: Option<String>,
}

impl Db {
    pub(crate) async fn create_user(
        &self,
        name: &str,
        project_id: Option<&str>,
        now: u64,
    ) -> io::Result<String> {
        if self.user_id_by_name(name).await?.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("user {name} already exists"),
            ));
        }

        let project_id = match project_id {
            Some(id) => Value::Text(id.to_string()),
            None => Value::Null,
        };
        let user_id = new_user_id();
        self.conn
            .execute(
                "INSERT INTO users (id, name, project_id, created_at) VALUES (?1, ?2, ?3, ?4)",
                (user_id.as_str(), name, project_id, now as i64),
            )
            .await
            .map_err(db_error)?;

        Ok(user_id)
    }

    pub(crate) async fn list_users(&self) -> io::Result<Vec<User>> {
        let mut rows = self
            .conn
            .query(
                "SELECT id, name, project_id, public_key, created_at FROM users ORDER BY created_at, name",
                (),
            )
            .await
            .map_err(db_error)?;

        let mut users = Vec::new();
        while let Some(row) = rows.next().await.map_err(db_error)? {
            users.push(User {
                id: text_column(&row, 0)?,
                name: text_column(&row, 1)?,
                project_id: optional_text_column(&row, 2)?,
                has_key: !matches!(row.get_value(3).map_err(db_error)?, Value::Null),
                created_at: integer_column(&row, 4)? as u64,
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
    /// Resolve a request's key to the public key that must verify it and the
    /// user it belongs to. A key id is a user id today (one key per user).
    pub(crate) async fn key_owner(&self, key_id: &str) -> io::Result<Option<(String, AuthUser)>> {
        let mut rows = self
            .conn
            .query(
                "SELECT public_key, id, project_id FROM users
                 WHERE id = ?1 AND public_key IS NOT NULL",
                params![key_id],
            )
            .await
            .map_err(db_error)?;

        match rows.next().await.map_err(db_error)? {
            Some(row) => Ok(Some((
                text_column(&row, 0)?,
                AuthUser {
                    id: text_column(&row, 1)?,
                    project_id: optional_text_column(&row, 2)?,
                },
            ))),
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
