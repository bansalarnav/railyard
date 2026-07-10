use libsql::params;
use std::io;

use super::{Db, db_error, integer_column, text_column};

pub(crate) struct Project {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) created_at: u64,
}

impl Db {
    pub(crate) async fn create_project(&self, name: &str, now: u64) -> io::Result<Project> {
        if self.project_id_by_name(name).await?.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("project {name} already exists"),
            ));
        }

        let project_id = new_project_id();
        self.conn
            .execute(
                "INSERT INTO projects (id, name, created_at) VALUES (?1, ?2, ?3)",
                (project_id.as_str(), name, now as i64),
            )
            .await
            .map_err(db_error)?;

        Ok(Project {
            id: project_id,
            name: name.to_string(),
            created_at: now,
        })
    }

    pub(crate) async fn list_projects(&self) -> io::Result<Vec<Project>> {
        let mut rows = self
            .conn
            .query(
                "SELECT id, name, created_at FROM projects ORDER BY created_at, name",
                (),
            )
            .await
            .map_err(db_error)?;

        let mut projects = Vec::new();
        while let Some(row) = rows.next().await.map_err(db_error)? {
            projects.push(Project {
                id: text_column(&row, 0)?,
                name: text_column(&row, 1)?,
                created_at: integer_column(&row, 2)? as u64,
            });
        }

        Ok(projects)
    }

    async fn project_id_by_name(&self, name: &str) -> io::Result<Option<String>> {
        let mut rows = self
            .conn
            .query("SELECT id FROM projects WHERE name = ?1", params![name])
            .await
            .map_err(db_error)?;

        match rows.next().await.map_err(db_error)? {
            Some(row) => Ok(Some(text_column(&row, 0)?)),
            None => Ok(None),
        }
    }
}

fn new_project_id() -> String {
    use rand::RngCore;

    let mut bytes = [0u8; 8];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    format!("prj_{}", hex::encode(bytes))
}
