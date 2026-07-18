use libsql::params;
use railyard_auth::DeploymentStatus;
use std::io;

use super::{Db, db_error, integer_column, optional_text_column, text_column};

pub(crate) struct Deployment {
    pub(crate) id: String,
    pub(crate) project_id: String,
    pub(crate) status: DeploymentStatus,
    pub(crate) error: Option<String>,
    pub(crate) created_at: u64,
    pub(crate) updated_at: u64,
}

impl Db {
    pub(crate) async fn create_deployment(
        &self,
        project_id: &str,
        status: DeploymentStatus,
        now: u64,
    ) -> io::Result<Deployment> {
        let id = new_deployment_id();
        self.conn
            .execute(
                "INSERT INTO deployments (id, project_id, status, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?4)",
                (id.as_str(), project_id, status.as_str(), now as i64),
            )
            .await
            .map_err(db_error)?;

        Ok(Deployment {
            id,
            project_id: project_id.to_string(),
            status,
            error: None,
            created_at: now,
            updated_at: now,
        })
    }

    pub(crate) async fn set_deployment_status(
        &self,
        id: &str,
        status: DeploymentStatus,
        error: Option<&str>,
        now: u64,
    ) -> io::Result<()> {
        self.conn
            .execute(
                "UPDATE deployments SET status = ?2, error = ?3, updated_at = ?4 WHERE id = ?1",
                params![id, status.as_str(), error, now as i64],
            )
            .await
            .map_err(db_error)?;
        Ok(())
    }

    pub(crate) async fn list_deployments(&self, project_id: &str) -> io::Result<Vec<Deployment>> {
        let mut rows = self
            .conn
            .query(
                "SELECT id, project_id, status, error, created_at, updated_at \
                 FROM deployments WHERE project_id = ?1 \
                 ORDER BY created_at DESC, id DESC",
                params![project_id],
            )
            .await
            .map_err(db_error)?;

        let mut deployments = Vec::new();
        while let Some(row) = rows.next().await.map_err(db_error)? {
            deployments.push(Deployment {
                id: text_column(&row, 0)?,
                project_id: text_column(&row, 1)?,
                status: status_column(&row, 2)?,
                error: optional_text_column(&row, 3)?,
                created_at: integer_column(&row, 4)? as u64,
                updated_at: integer_column(&row, 5)? as u64,
            });
        }

        Ok(deployments)
    }
}

fn status_column(row: &libsql::Row, index: i32) -> io::Result<DeploymentStatus> {
    let text = text_column(row, index)?;
    DeploymentStatus::parse(&text)
        .ok_or_else(|| io::Error::other(format!("unknown deployment status {text:?}")))
}

fn new_deployment_id() -> String {
    use rand::RngCore;

    let mut bytes = [0u8; 8];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    format!("dpl_{}", hex::encode(bytes))
}
