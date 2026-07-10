use async_trait::async_trait;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router, middleware, routing::get};
use pingora::server::ShutdownWatch;
use pingora::services::ServiceReadyNotifier;
use pingora::services::background::BackgroundService;
use railyard_auth::{
    CreateProjectRequest, ListProjectsResponse, PROJECTS_PATH, ProjectSummary, REDEEM_INVITE_PATH,
    unix_timestamp,
};
use serde::Serialize;
use std::collections::HashMap;
use std::io;
use std::sync::{Arc, Mutex};

use super::auth::{redeem_invite, verify_signature};
use super::state::{ApiState, AppState};
use crate::db::{Db, Project};

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
        mut shutdown: ShutdownWatch,
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

        ready_notifier.notify_ready();

        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = shutdown.changed().await;
            })
            .await
            .expect("API service exited with error");
    }
}

fn api_routes(state: &ApiState) -> Router<ApiState> {
    let protected = Router::new()
        .route("/", get(root))
        .route("/api/services", get(list_services))
        .route(PROJECTS_PATH, get(list_projects).post(create_project))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            verify_signature,
        ));
    protected.route(REDEEM_INVITE_PATH, post(redeem_invite))
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
