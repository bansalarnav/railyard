use async_trait::async_trait;
use axum::middleware;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::{Json, Router, routing::get};
use pingora::server::ShutdownWatch;
use pingora::services::ServiceReadyNotifier;
use pingora::services::background::BackgroundService;
use serde::Serialize;

use crate::app::APP_NAME;
use crate::auth::require_signed_request;
use crate::state::{AppState, requested_service};

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
    app_name: &'static str,
    base_domain: String,
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
        let protected_api = Router::new()
            .route("/api/services", get(list_services))
            .route("/admin/api/services", get(list_services))
            .route_layer(middleware::from_fn_with_state(
                self.state.clone(),
                require_signed_request,
            ));

        let app = Router::new()
            .route("/", get(root))
            .route("/admin", get(root))
            .route("/healthz", get(healthz))
            .merge(protected_api)
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

async fn root(State(state): State<AppState>, headers: HeaderMap) -> String {
    if let Some(service) = requested_service(&headers, &state.base_domain) {
        if let Some(addr) = state.service_upstreams.get(&service) {
            return format!(
                "Service {service} is configured for proxying to {addr}. Requests to this host should be served by the proxied container."
            );
        }

        return format!(
            "Service {service} is not configured yet. Add CONTAINER_UPSTREAM_{} to route it through Pingora.",
            service.to_ascii_uppercase().replace('-', "_")
        );
    }

    format!(
        "Aethon API is running.\nproxy={}\napi={}\nservices={}",
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
        app_name: APP_NAME,
        base_domain: state.base_domain.clone(),
        proxy_addr: state.proxy_addr.to_string(),
        api_addr: state.api_addr.to_string(),
        services,
    })
}
