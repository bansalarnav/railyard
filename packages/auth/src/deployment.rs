use serde::{Deserialize, Serialize};
use std::fmt;

use crate::PROJECTS_PATH;

/// `POST` a gzipped tarball of the project source here to create a
/// deployment; `GET` lists the project's deployments, newest first.
pub fn project_deployments_path(project_id: &str) -> String {
    format!("{PROJECTS_PATH}/{project_id}/deployments")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentStatus {
    /// Archive received; the server is unpacking it.
    Unpacking,
    /// Source is unpacked on the server, ready for the next stage.
    Ready,
    Failed,
}

impl DeploymentStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unpacking => "unpacking",
            Self::Ready => "ready",
            Self::Failed => "failed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "unpacking" => Some(Self::Unpacking),
            "ready" => Some(Self::Ready),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

impl fmt::Display for DeploymentStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeploymentSummary {
    pub id: String,
    pub project_id: String,
    pub status: DeploymentStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ListDeploymentsResponse {
    pub deployments: Vec<DeploymentSummary>,
}
