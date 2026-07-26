//! The HTTP contract between the CLI and the server: request and response
//! bodies, plus the route paths they are sent to. Signing lives in
//! `railyard-auth`.

mod project;
mod release;
mod user;

pub use project::{CreateProjectRequest, ListProjectsResponse, PROJECTS_PATH, ProjectSummary};
pub use release::{ListReleasesResponse, ReleaseStatus, ReleaseSummary, project_releases_path};
pub use user::{
    CreateUserRequest, CreateUserResponse, ListUsersResponse, USERS_PATH, UserSummary, WHOAMI_PATH,
    WhoamiResponse,
};
