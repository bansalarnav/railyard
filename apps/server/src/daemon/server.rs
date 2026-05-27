use pingora::proxy::http_proxy_service;
use pingora::server::Server;
use pingora::server::configuration::{Opt, ServerConf};
use pingora::services::background::background_service;
use std::path::Path;

use crate::app::APP_NAME;

use super::api::ApiService;
use super::proxy::{ControlPlaneProxy, RoutingTable};
use super::state::AppState;

pub(super) fn run_server(daemon: bool, pid_file: &Path, upgrade_sock: &Path) {
    let state = AppState::load();
    let proxy_addr = state.proxy_addr;
    let api_addr = state.api_addr;
    let routes = RoutingTable::from_state(&state);

    let opt = pingora_opt(daemon);
    let conf = pingora_conf(daemon, pid_file, upgrade_sock);
    let mut server = Server::new_with_opt_and_conf(Some(opt), conf);
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

fn pingora_conf(daemon: bool, pid_file: &Path, upgrade_sock: &Path) -> ServerConf {
    let mut conf = ServerConf::new().expect("failed to create pingora config");
    conf.daemon = daemon;
    conf.pid_file = pid_file.to_string_lossy().into_owned();
    conf.upgrade_sock = upgrade_sock.to_string_lossy().into_owned();
    conf
}
