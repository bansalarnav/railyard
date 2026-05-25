use pingora::proxy::http_proxy_service;
use pingora::server::Server;
use pingora::server::configuration::Opt;
use pingora::services::background::background_service;

use crate::api::ApiService;
use crate::app::APP_NAME;
use crate::proxy::{ControlPlaneProxy, RoutingTable};
use crate::state::AppState;

pub(super) fn run_server(daemon: bool) {
    let state = AppState::load();
    let proxy_addr = state.proxy_addr;
    let api_addr = state.api_addr;
    let routes = RoutingTable::from_state(&state);

    let opt = pingora_opt(daemon);
    let mut server = Server::new(Some(opt)).expect("failed to create pingora server");
    server.bootstrap();

    let api_handle = server.add_service(background_service(
        "api",
        ApiService {
            state: state.clone(),
        },
    ));

    let mut proxy_service = http_proxy_service(&server.configuration, ControlPlaneProxy { routes });
    proxy_service.add_tcp(&proxy_addr.to_string());

    let proxy_handle = server.add_service(proxy_service);
    proxy_handle.add_dependency(&api_handle);

    println!(
        "Starting {} hybrid ingress on http://{}",
        APP_NAME, proxy_addr
    );
    println!("Internal API bound to http://{}", api_addr);
    println!("Dashboard URL: http://{}", proxy_addr);

    server.run_forever();
}

fn pingora_opt(daemon: bool) -> Opt {
    Opt {
        daemon,
        ..Default::default()
    }
}
