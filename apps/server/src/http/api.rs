use async_trait::async_trait;
use axum::extract::State;
use axum::{Json, Router, routing::get};
use pingora::server::ShutdownWatch;
use pingora::services::ServiceReadyNotifier;
use pingora::services::background::BackgroundService;
use serde::Serialize;

use super::state::AppState;

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
        // The proxy forwards `railyard.*` hosts with the path untouched and
        // `/railyard/...` paths with the prefix intact, so serve the same
        // routes at the root and under /railyard.
        let app = api_routes()
            .nest("/railyard", api_routes())
            .route("/healthz", get(healthz))
            .with_state(self.state.clone());

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

fn api_routes() -> Router<AppState> {
    Router::new()
        .route("/", get(root))
        .route("/api/services", get(list_services))
}

async fn root(State(state): State<AppState>) -> String {
    format!(
        "Railyard API is running.\nproxy={}\napi={}\nservices={}",
        state.proxy_addr,
        state.api_addr,
        state.service_upstreams.len()
    )
}

async fn healthz() -> &'static str {
    "ok"
}

async fn list_services(State(state): State<AppState>) -> Json<ServicesResponse> {
    let services = state
        .service_upstreams
        .iter()
        .map(|(name, addr)| ServiceEntry {
            name: name.clone(),
            upstream_addr: addr.to_string(),
        })
        .collect();

    Json(ServicesResponse {
        proxy_addr: state.proxy_addr.to_string(),
        api_addr: state.api_addr.to_string(),
        services,
    })
}
