use async_trait::async_trait;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, post};
use axum::{Extension, Json, Router, middleware, routing::get};
use pingora::server::ShutdownWatch;
use pingora::services::ServiceReadyNotifier;
use pingora::services::background::BackgroundService;
use railyard_auth::{
    CreateProjectRequest, CreateUserRequest, CreateUserResponse, InviteProject,
    ListProjectsResponse, ListUsersResponse, PROJECTS_PATH, ProjectSummary, REDEEM_INVITE_PATH,
    USERS_PATH, UserSummary, WHOAMI_PATH, WhoamiResponse, unix_timestamp,
};
use serde::Serialize;
use std::collections::HashMap;
use std::io;
use std::sync::{Arc, Mutex};

use super::auth::{redeem_invite, verify_signature};
use super::state::{ApiState, AppState};
use crate::db::{AuthUser, Db, Project};
use crate::invite::mint_invite;

pub(crate) struct ApiService {
    pub(crate) state: AppState,
}

#[derive(Serialize)]
struct ServiceEntry {
    name: String,
    upstream_addr: String,
}

#[derive(Serialize)]
struct ServicesResponse {
    proxy_addr: String,
    api_addr: String,
    services: Vec<ServiceEntry>,
}

#[async_trait]
impl BackgroundService for ApiService {
    async fn start_with_ready_notifier(
        &self,
        shutdown: ShutdownWatch,
        ready_notifier: ServiceReadyNotifier,
    ) {
        let db = Db::open().await.expect("failed to open auth database");
        let state = ApiState {
            app: self.state.clone(),
            db: Arc::new(db),
            seen_nonces: Arc::new(Mutex::new(HashMap::new())),
        };
        let app = api_routes(&state)
            .nest("/railyard", api_routes(&state))
            .route("/healthz", get(healthz))
            .with_state(state.clone());

        let listener = tokio::net::TcpListener::bind(self.state.api_addr)
            .await
            .expect("failed to bind internal API listener");
        let admin_listener = bind_admin_socket();

        ready_notifier.notify_ready();

        let mut tcp_shutdown = shutdown.clone();
        let tcp = axum::serve(listener, app).with_graceful_shutdown(async move {
            let _ = tcp_shutdown.changed().await;
        });
        let mut admin_shutdown = shutdown.clone();
        let admin = axum::serve(admin_listener, admin_routes(&state)).with_graceful_shutdown(
            async move {
                let _ = admin_shutdown.changed().await;
            },
        );

        let (tcp, admin) = tokio::join!(tcp, admin);
        tcp.expect("API service exited with error");
        admin.expect("admin socket service exited with error");
    }
}

/// The local admin API: the server CLI's line to the daemon. Only the
/// machine's admin can reach the socket (0600), so requests skip signature
/// verification and act as an admin user.
fn bind_admin_socket() -> tokio::net::UnixListener {
    use std::os::unix::fs::PermissionsExt;

    let path = crate::paths::admin_sock_path();
    let _ = std::fs::remove_file(&path);
    let listener =
        tokio::net::UnixListener::bind(&path).expect("failed to bind admin socket listener");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
        .expect("failed to restrict admin socket permissions");
    listener
}

/// Requests on the admin socket act as an admin user without signatures;
/// the socket's file permissions are the trust boundary.
fn admin_routes(state: &ApiState) -> Router {
    protected_routes()
        .layer(Extension(AuthUser {
            id: "local".to_string(),
            name: "local".to_string(),
            project_id: None,
        }))
        .with_state(state.clone())
}

fn api_routes(state: &ApiState) -> Router<ApiState> {
    protected_routes()
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            verify_signature,
        ))
        .route(REDEEM_INVITE_PATH, post(redeem_invite))
}

/// Every authenticated route, shared by the signed TCP listener and the
/// local admin socket. Handlers see the caller as an `AuthUser` extension,
/// inserted by the signature middleware or the admin socket respectively.
fn protected_routes() -> Router<ApiState> {
    Router::new()
        .route("/", get(root))
        .route("/api/services", get(list_services))
        .route(PROJECTS_PATH, get(list_projects).post(create_project))
        .route(USERS_PATH, get(list_users).post(create_user))
        .route(&format!("{USERS_PATH}/{{name}}"), delete(remove_user))
        .route(WHOAMI_PATH, get(whoami))
}

/// Echo back who the verified key belongs to — the client stores only a key
/// id locally, so this is how it learns (and proves) its own identity.
async fn whoami(
    State(state): State<ApiState>,
    Extension(caller): Extension<AuthUser>,
) -> Response {
    let project_name = match &caller.project_id {
        None => None,
        Some(id) => match state.db.project_by_id(id).await {
            Ok(project) => project.map(|project| project.name),
            Err(error) => {
                log::error!("project lookup failed: {error}");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        },
    };

    Json(WhoamiResponse {
        user_id: caller.id,
        name: caller.name,
        project_id: caller.project_id,
        project_name,
    })
    .into_response()
}

async fn root(State(state): State<ApiState>) -> String {
    format!(
        "Railyard API is running.\nproxy={}\napi={}\nservices={}",
        state.app.proxy_addr,
        state.app.api_addr,
        state.app.service_upstreams.len()
    )
}

async fn healthz() -> &'static str {
    "ok"
}

async fn list_services(State(state): State<ApiState>) -> Json<ServicesResponse> {
    let services = state
        .app
        .service_upstreams
        .iter()
        .map(|(name, addr)| ServiceEntry {
            name: name.clone(),
            upstream_addr: addr.to_string(),
        })
        .collect();

    Json(ServicesResponse {
        proxy_addr: state.app.proxy_addr.to_string(),
        api_addr: state.app.api_addr.to_string(),
        services,
    })
}

async fn create_project(
    State(state): State<ApiState>,
    Json(request): Json<CreateProjectRequest>,
) -> Response {
    if !is_valid_project_name(&request.name) {
        return (
            StatusCode::BAD_REQUEST,
            "project name must be a lowercase DNS label (a-z, 0-9, hyphens; max 63 chars)",
        )
            .into_response();
    }

    match state
        .db
        .create_project(&request.name, unix_timestamp())
        .await
    {
        Ok(project) => Json(project_summary(project)).into_response(),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            (StatusCode::CONFLICT, error.to_string()).into_response()
        }
        Err(error) => {
            log::error!("project creation failed: {error}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// Create a user (project-scoped, or a server-wide admin when `project_id`
/// is absent) and mint its invite blob. Only admins may invite — a
/// project-scoped inviter gets a 403 regardless of the requested scope.
async fn create_user(
    State(state): State<ApiState>,
    Extension(inviter): Extension<AuthUser>,
    Json(request): Json<CreateUserRequest>,
) -> Response {
    if inviter.project_id.is_some() {
        return (
            StatusCode::FORBIDDEN,
            "only server admins can create users and invites",
        )
            .into_response();
    }

    let project = match &request.project_id {
        None => None,
        Some(id) => match state.db.project_by_id(id).await {
            Ok(Some(project)) => Some(InviteProject {
                id: project.id,
                name: project.name,
            }),
            Ok(None) => {
                return (
                    StatusCode::NOT_FOUND,
                    format!("no project {id} on this server"),
                )
                    .into_response();
            }
            Err(error) => {
                log::error!("project lookup failed: {error}");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        },
    };

    match mint_invite(&state.db, &request.name, project).await {
        Ok(minted) => {
            log::info!(
                "admin {} created user {} ({})",
                inviter.id,
                request.name,
                minted.user_id
            );
            Json(CreateUserResponse {
                user_id: minted.user_id,
                invite_blob: minted.blob,
                expires_at: minted.expires_at,
            })
            .into_response()
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            (StatusCode::CONFLICT, error.to_string()).into_response()
        }
        Err(error) if error.kind() == io::ErrorKind::InvalidInput => {
            (StatusCode::BAD_REQUEST, error.to_string()).into_response()
        }
        Err(error) => {
            log::error!("user creation failed: {error}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn list_users(
    State(state): State<ApiState>,
    Extension(caller): Extension<AuthUser>,
) -> Response {
    if caller.project_id.is_some() {
        return (StatusCode::FORBIDDEN, "only server admins can list users").into_response();
    }

    match state.db.list_users().await {
        Ok(users) => Json(ListUsersResponse {
            users: users
                .into_iter()
                .map(|user| UserSummary {
                    id: user.id,
                    name: user.name,
                    project_id: user.project_id,
                    has_key: user.has_key,
                    created_at: user.created_at,
                })
                .collect(),
        })
        .into_response(),
        Err(error) => {
            log::error!("user listing failed: {error}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn remove_user(
    State(state): State<ApiState>,
    Extension(caller): Extension<AuthUser>,
    Path(name): Path<String>,
) -> Response {
    if caller.project_id.is_some() {
        return (StatusCode::FORBIDDEN, "only server admins can remove users").into_response();
    }

    match state.db.remove_user(&name).await {
        Ok(true) => {
            log::info!("admin {} removed user {name}", caller.id);
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => (StatusCode::NOT_FOUND, format!("no user named {name}")).into_response(),
        Err(error) => {
            log::error!("user removal failed: {error}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn list_projects(State(state): State<ApiState>) -> Response {
    match state.db.list_projects().await {
        Ok(projects) => Json(ListProjectsResponse {
            projects: projects.into_iter().map(project_summary).collect(),
        })
        .into_response(),
        Err(error) => {
            log::error!("project listing failed: {error}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

fn project_summary(project: Project) -> ProjectSummary {
    ProjectSummary {
        id: project.id,
        name: project.name,
        created_at: project.created_at,
    }
}

/// Mirrors the manifest's `project.name` rule — the name ends up in
/// generated hostnames, so the server enforces the same DNS-label shape.
fn is_valid_project_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 63
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !name.starts_with('-')
        && !name.ends_with('-')
}
