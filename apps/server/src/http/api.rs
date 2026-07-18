use async_trait::async_trait;
use axum::body::Body;
use axum::extract::{Path, Query, Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, post};
use axum::{Extension, Json, Router, middleware, routing::get};
use flate2::read::GzDecoder;
use pingora::server::ShutdownWatch;
use pingora::services::ServiceReadyNotifier;
use pingora::services::background::BackgroundService;
use railyard_auth::{
    CreateProjectRequest, CreateUserRequest, CreateUserResponse, DeploymentStatus,
    DeploymentSummary, InviteProject, ListDeploymentsResponse, ListProjectsResponse,
    ListUsersResponse, PROJECTS_PATH, ProjectSummary, REDEEM_INVITE_PATH, USERS_PATH, UserSummary,
    WHOAMI_PATH, WhoamiResponse, unix_timestamp,
};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io;
use std::sync::{Arc, Mutex};

use super::auth::{SignedContentHash, redeem_invite, verify_body_hash, verify_signature};
use super::state::{ApiState, AppState};
use crate::db::{AuthUser, Db, Deployment, Project};
use crate::invite::mint_invite;

pub(crate) struct ApiService {
    pub(crate) state: AppState,
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
        let admin =
            axum::serve(admin_listener, admin_routes(&state)).with_graceful_shutdown(async move {
                let _ = admin_shutdown.changed().await;
            });

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
///
/// The deployments route sits outside the `verify_body_hash` layer: uploads
/// stream the body to disk and check the signed hash there instead of
/// buffering it in memory.
fn protected_routes() -> Router<ApiState> {
    Router::new()
        .route("/", get(root))
        .route(PROJECTS_PATH, get(list_projects).post(create_project))
        .route(USERS_PATH, get(list_users).post(create_user))
        .route(&format!("{USERS_PATH}/{{name}}"), delete(remove_user))
        .route(WHOAMI_PATH, get(whoami))
        .route_layer(middleware::from_fn(verify_body_hash))
        .route(
            &format!("{PROJECTS_PATH}/{{project_id}}/deployments"),
            get(list_deployments).post(create_deployment),
        )
}

/// Echo back who the verified key belongs to — the client stores only a key
/// id locally, so this is how it learns (and proves) its own identity.
async fn whoami(State(state): State<ApiState>, Extension(caller): Extension<AuthUser>) -> Response {
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

async fn create_project(
    State(state): State<ApiState>,
    Extension(caller): Extension<AuthUser>,
    Json(request): Json<CreateProjectRequest>,
) -> Response {
    if caller.project_id.is_some() {
        return (
            StatusCode::FORBIDDEN,
            "only server admins can create projects",
        )
            .into_response();
    }
    if !is_valid_project_name(&request.name) {
        return (
            StatusCode::BAD_REQUEST,
            "project name must be a lowercase DNS label (a-z, 0-9, hyphens; max 63 chars)",
        )
            .into_response();
    }
    if let Some(id) = &request.id
        && !is_valid_project_id(id)
    {
        return (
            StatusCode::BAD_REQUEST,
            "project id must look like prj_<16 hex chars>",
        )
            .into_response();
    }

    match state
        .db
        .create_project(&request.name, request.id.as_deref(), unix_timestamp())
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

async fn list_projects(
    State(state): State<ApiState>,
    Extension(caller): Extension<AuthUser>,
) -> Response {
    match state.db.list_projects().await {
        // A project-scoped key sees exactly its own project, nothing else.
        Ok(projects) => Json(ListProjectsResponse {
            projects: projects
                .into_iter()
                .filter(|project| match &caller.project_id {
                    None => true,
                    Some(scope) => project.id == *scope,
                })
                .map(project_summary)
                .collect(),
        })
        .into_response(),
        Err(error) => {
            log::error!("project listing failed: {error}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// Streamed uploads are capped by disk, not memory, so the limit is about
/// runaway requests rather than archive size.
const MAX_ARCHIVE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// The message rides the query string so the body stays a bare archive; being
/// part of the signed path-and-query, it needs no extra verification.
#[derive(serde::Deserialize)]
struct CreateDeploymentQuery {
    message: Option<String>,
}

/// Receive a gzipped tarball of the project's source and unpack it under the
/// server's deployments directory. Every upload becomes a deployment row, so
/// each `up` run is tracked even when receiving or unpacking fails.
async fn create_deployment(
    State(state): State<ApiState>,
    Extension(caller): Extension<AuthUser>,
    Path(project_id): Path<String>,
    Query(query): Query<CreateDeploymentQuery>,
    signed_hash: Option<Extension<SignedContentHash>>,
    request: Request,
) -> Response {
    if let Err(response) = confirm_project_access(&state, &caller, &project_id).await {
        return response;
    }

    let message = query.message.as_deref().filter(|text| !text.is_empty());
    let deployment = match state
        .db
        .create_deployment(
            &project_id,
            DeploymentStatus::Unpacking,
            message,
            unix_timestamp(),
        )
        .await
    {
        Ok(deployment) => deployment,
        Err(error) => {
            log::error!("deployment creation failed: {error}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let dir = crate::paths::deployment_dir(&project_id, &deployment.id);
    let signed_hash = signed_hash.map(|Extension(hash)| hash.0);
    let unpacked = match receive_archive(&dir, request.into_body(), signed_hash).await {
        Ok(()) => {
            let dir = dir.clone();
            tokio::task::spawn_blocking(move || unpack_archive(&dir))
                .await
                .map_err(io::Error::other)
                .and_then(|result| result)
        }
        Err(error) => Err(error),
    };

    let (status, error) = match &unpacked {
        Ok(()) => (DeploymentStatus::Ready, None),
        Err(error) => (DeploymentStatus::Failed, Some(error.to_string())),
    };
    let now = unix_timestamp();
    if let Err(error) = state
        .db
        .set_deployment_status(&deployment.id, status, error.as_deref(), now)
        .await
    {
        log::error!("deployment status update failed: {error}");
    }

    match unpacked {
        Ok(()) => {
            log::info!(
                "user {} created deployment {} for project {project_id}",
                caller.id,
                deployment.id
            );
            Json(DeploymentSummary {
                id: deployment.id,
                project_id,
                status,
                message: deployment.message,
                error,
                created_at: deployment.created_at,
                updated_at: now,
            })
            .into_response()
        }
        Err(failure) => (
            StatusCode::BAD_REQUEST,
            format!("deployment {} failed: {failure}", deployment.id),
        )
            .into_response(),
    }
}

async fn list_deployments(
    State(state): State<ApiState>,
    Extension(caller): Extension<AuthUser>,
    Path(project_id): Path<String>,
) -> Response {
    if let Err(response) = confirm_project_access(&state, &caller, &project_id).await {
        return response;
    }

    match state.db.list_deployments(&project_id).await {
        Ok(deployments) => Json(ListDeploymentsResponse {
            deployments: deployments.into_iter().map(deployment_summary).collect(),
        })
        .into_response(),
        Err(error) => {
            log::error!("deployment listing failed: {error}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// A project-scoped key may only touch its own project; admins may touch
/// any. Also confirms the project exists, which keeps unknown ids out of
/// the deployments table and the filesystem.
async fn confirm_project_access(
    state: &ApiState,
    caller: &AuthUser,
    project_id: &str,
) -> Result<(), Response> {
    if caller
        .project_id
        .as_deref()
        .is_some_and(|scope| scope != project_id)
    {
        return Err((StatusCode::FORBIDDEN, "key is not scoped to this project").into_response());
    }

    match state.db.project_by_id(project_id).await {
        Ok(Some(_)) => Ok(()),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            format!("no project {project_id} on this server"),
        )
            .into_response()),
        Err(error) => {
            log::error!("project lookup failed: {error}");
            Err(StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
    }
}

/// Stream the request body to <dir>/archive.tar.gz, hashing bytes as they
/// land. The signature only vouches for the hash the client claimed, so the
/// archive is untrusted until the streamed bytes match that claim.
async fn receive_archive(
    dir: &std::path::Path,
    body: Body,
    signed_hash: Option<String>,
) -> io::Result<()> {
    use http_body_util::BodyExt;
    use tokio::io::AsyncWriteExt;

    tokio::fs::create_dir_all(dir).await?;
    let mut file =
        tokio::io::BufWriter::new(tokio::fs::File::create(dir.join("archive.tar.gz")).await?);
    let mut hasher = Sha256::new();
    let mut received: u64 = 0;
    let mut body = body;
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(io::Error::other)?;
        let Ok(data) = frame.into_data() else {
            continue;
        };
        received += data.len() as u64;
        if received > MAX_ARCHIVE_BYTES {
            return Err(io::Error::other(format!(
                "archive exceeds the {MAX_ARCHIVE_BYTES} byte limit"
            )));
        }
        hasher.update(&data);
        file.write_all(&data).await?;
    }
    file.flush().await?;

    if received == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "request body must be a gzipped tarball of the project source",
        ));
    }
    if let Some(expected) = signed_hash
        && hex::encode(hasher.finalize()) != expected
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "body does not match the signed content hash",
        ));
    }
    Ok(())
}

/// Keep the uploaded archive next to its unpacked tree so a bad unpack can
/// be inspected later: <dir>/archive.tar.gz and <dir>/source/.
fn unpack_archive(dir: &std::path::Path) -> io::Result<()> {
    let archive = std::fs::File::open(dir.join("archive.tar.gz"))?;
    let source = dir.join("source");
    std::fs::create_dir_all(&source)?;
    tar::Archive::new(GzDecoder::new(io::BufReader::new(archive))).unpack(&source)
}

fn deployment_summary(deployment: Deployment) -> DeploymentSummary {
    DeploymentSummary {
        id: deployment.id,
        project_id: deployment.project_id,
        status: deployment.status,
        message: deployment.message,
        error: deployment.error,
        created_at: deployment.created_at,
        updated_at: deployment.updated_at,
    }
}

fn project_summary(project: Project) -> ProjectSummary {
    ProjectSummary {
        id: project.id,
        name: project.name,
        created_at: project.created_at,
    }
}

/// Only ids this platform could have minted are accepted for reuse.
fn is_valid_project_id(id: &str) -> bool {
    id.strip_prefix("prj_").is_some_and(|hex| {
        hex.len() == 16
            && hex
                .chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
    })
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
