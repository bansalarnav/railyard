use pingora::proxy::http_proxy_service;
use pingora::server::Server;
use pingora::server::configuration::Opt;
use pingora::services::background::background_service;

use crate::app::APP_NAME;
use crate::control_plane::AxumControlPlane;
use crate::proxy::{ControlPlaneProxy, RoutingTable};
use crate::state::{AppState, display_url};

use super::layout::runtime_conf_path;

pub(super) fn run_server(daemon: bool) {
    let state = AppState::load();
    let proxy_addr = state.proxy_addr;
    let axum_addr = state.axum_addr;
    let base_domain = state.base_domain.clone();
    let routes = RoutingTable::from_state(&state);

    let opt = pingora_opt(daemon);
    let mut server = Server::new(Some(opt)).expect("failed to create pingora server");
    server.bootstrap();

    let axum_handle = server.add_service(background_service(
        "axum-control-plane",
        AxumControlPlane {
            state: state.clone(),
        },
    ));

    let mut proxy_service = http_proxy_service(&server.configuration, ControlPlaneProxy { routes });
    proxy_service.add_tcp(&proxy_addr.to_string());

    let proxy_handle = server.add_service(proxy_service);
    proxy_handle.add_dependency(&axum_handle);

    println!(
        "Starting {} hybrid ingress on http://{}",
        APP_NAME, proxy_addr
    );
    println!("Internal control plane bound to http://{}", axum_addr);
    println!(
        "Dashboard URL: {}",
        display_url(base_domain.as_str(), proxy_addr.port())
    );
    println!(
        "Example deployment URL: http://howdy.{}:{}",
        base_domain,
        proxy_addr.port()
    );
    println!("Register container routes with env like CONTAINER_UPSTREAM_HOWDY=127.0.0.1:4001");

    server.run_forever();
}

fn pingora_opt(daemon: bool) -> Opt {
    if daemon {
        Opt {
            daemon: true,
            conf: Some(runtime_conf_path().display().to_string()),
            ..Default::default()
        }
    } else {
        Default::default()
    }
}
