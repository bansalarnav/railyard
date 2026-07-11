use serde::{Deserialize, Serialize};

pub const PROJECTS_PATH: &str = "/api/projects";

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateProjectRequest {
    pub name: String,
    /// Keep an id the manifest already carries, so one project stays one
    /// project across servers; absent means the server mints a fresh id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProjectSummary {
    pub id: String,
    pub name: String,
    pub created_at: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ListProjectsResponse {
    pub projects: Vec<ProjectSummary>,
}
