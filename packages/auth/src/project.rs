use serde::{Deserialize, Serialize};

pub const PROJECTS_PATH: &str = "/api/projects";

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateProjectRequest {
    pub name: String,
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
