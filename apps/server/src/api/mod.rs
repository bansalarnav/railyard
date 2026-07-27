//! The Railyard API: users, invites, projects and releases.
//!
//! It listens on loopback behind the ingress proxy, plus a local admin
//! socket that the server CLI uses. Nothing here knows about routing or
//! process supervision — that is the proxy's side of the server.

mod auth;
mod db;
mod invite;
mod routes;

use async_trait::async_trait;
use pingora::server::ShutdownWatch;
use pingora::services::ServiceReadyNotifier;
use pingora::services::background::BackgroundService;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::config::Config;
use db::Db;

pub(crate) struct ApiService {
    pub(crate) config: Config,
}

#[derive(Clone)]
pub(crate) struct ApiState {
    pub(crate) config: Config,
    pub(crate) db: Arc<Db>,
    pub(crate) seen_nonces: Arc<Mutex<HashMap<String, u64>>>,
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
            config: self.config.clone(),
            db: Arc::new(db),
            seen_nonces: Arc::new(Mutex::new(HashMap::new())),
        };

        let listener = tokio::net::TcpListener::bind(self.config.api_addr)
            .await
            .expect("failed to bind internal API listener");
        let admin_listener = bind_admin_socket();

        ready_notifier.notify_ready();

        let mut tcp_shutdown = shutdown.clone();
        let tcp = axum::serve(listener, routes::signed_routes(&state)).with_graceful_shutdown(
            async move {
                let _ = tcp_shutdown.changed().await;
            },
        );
        let mut admin_shutdown = shutdown.clone();
        let admin = axum::serve(admin_listener, routes::admin_routes(&state))
            .with_graceful_shutdown(async move {
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
