//! The HTTP contract between the CLI and the server: request and response
//! bodies, plus the route paths they are sent to. Signing lives in
//! `railyard-auth`.

mod deployment;
mod project;
mod user;

pub use deployment::{
    DeploymentStatus, DeploymentSummary, ListDeploymentsResponse, project_deployments_path,
};
pub use project::{CreateProjectRequest, ListProjectsResponse, PROJECTS_PATH, ProjectSummary};
pub use user::{
    CreateUserRequest, CreateUserResponse, ListUsersResponse, USERS_PATH, UserSummary, WHOAMI_PATH,
    WhoamiResponse,
};
