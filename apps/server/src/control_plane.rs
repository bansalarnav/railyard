use async_trait::async_trait;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::{Json, Router, routing::get};
use pingora::server::ShutdownWatch;
use pingora::services::ServiceReadyNotifier;
use pingora::services::background::BackgroundService;
use serde::Serialize;

use crate::state::{AppState, requested_service};

pub(crate) struct AxumControlPlane {
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
    axum_addr: String,
    services: Vec<ServiceEntry>,
}

#[async_trait]
impl BackgroundService for AxumControlPlane {
    async fn start_with_ready_notifier(
        &self,
        mut shutdown: ShutdownWatch,
        ready_notifier: ServiceReadyNotifier,
    ) {
        let app = Router::new()
            .route("/", get(root))
            .route("/healthz", get(healthz))
            .route("/api/services", get(list_services))
            .with_state(self.state.clone());

        let listener = tokio::net::TcpListener::bind(self.state.axum_addr)
            .await
            .expect("failed to bind internal axum listener");

        ready_notifier.notify_ready();

        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = shutdown.changed().await;
            })
            .await
            .expect("axum control plane exited with error");
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
        "Aethon control plane is running.\nproxy={}\naxum={}\nservices={}",
        state.proxy_addr,
        state.axum_addr,
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
        app_name: config::AppConfig::default().app_name,
        base_domain: state.base_domain.clone(),
        proxy_addr: state.proxy_addr.to_string(),
        axum_addr: state.axum_addr.to_string(),
        services,
    })
}
