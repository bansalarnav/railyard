use serde::{Deserialize, Serialize};

pub const USERS_PATH: &str = "/api/users";

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateUserRequest {
    pub name: String,
    /// Scope the new user to one project; absent creates a server-wide admin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateUserResponse {
    pub user_id: String,
    pub invite_blob: String,
    pub expires_at: u64,
}
