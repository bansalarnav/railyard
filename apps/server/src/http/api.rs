use async_trait::async_trait;
use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router, middleware, routing::get};
use pingora::server::ShutdownWatch;
use pingora::services::ServiceReadyNotifier;
use pingora::services::background::BackgroundService;
use railyard_auth::REDEEM_INVITE_PATH;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::auth::{redeem_invite, verify_signature};
use super::state::{ApiState, AppState};
use crate::db::Db;

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

        // The proxy forwards `railyard.*` hosts with the path untouched and
        // `/railyard/...` paths with the prefix intact, so serve the same
        // routes at the root and under /railyard.
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
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            verify_signature,
        ));

    // Invite redemption is the one unauthenticated endpoint: it is how a
    // client gets a key in the first place.
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
