use pingora::proxy::http_proxy_service;
use pingora::server::Server;
use pingora::server::configuration::{Opt, ServerConf};
use pingora::services::background::background_service;
use std::io;
use std::path::Path;

use super::api::ApiService;
use super::proxy::{IngressProxy, RoutingTable};
use super::state::AppState;

pub(crate) fn run_server(daemon: bool, pid_file: &Path, upgrade_sock: &Path) -> io::Result<()> {
    let state = AppState::load()?;
    let proxy_addr = state.proxy_addr;
    let api_addr = state.api_addr;
    let routes = RoutingTable::from_state(&state);

    let opt = Opt {
        daemon,
        ..Default::default()
    };
    let conf = pingora_conf(daemon, pid_file, upgrade_sock)?;
    let mut server = Server::new_with_opt_and_conf(Some(opt), conf);
    server.bootstrap();

    let api_handle = server.add_service(background_service("api", ApiService { state }));

    let mut proxy_service = http_proxy_service(&server.configuration, IngressProxy { routes });
    proxy_service.add_tcp(&proxy_addr.to_string());

    let proxy_handle = server.add_service(proxy_service);
    proxy_handle.add_dependency(&api_handle);

    println!("Starting railyard ingress on http://{proxy_addr}");
    println!("Internal API bound to http://{api_addr}");
    println!("Dashboard URL: http://{proxy_addr}/railyard");

    server.run_forever()
}

fn pingora_conf(daemon: bool, pid_file: &Path, upgrade_sock: &Path) -> io::Result<ServerConf> {
    let mut conf =
        ServerConf::new().ok_or_else(|| io::Error::other("failed to create pingora config"))?;
    conf.daemon = daemon;
    conf.pid_file = pid_file.to_string_lossy().into_owned();
    conf.upgrade_sock = upgrade_sock.to_string_lossy().into_owned();
    // Pingora's default grace period is 5 minutes, which would make `down`
    // leave the process draining long after its wait loop gives up.
    conf.grace_period_seconds = Some(1);
    conf.graceful_shutdown_timeout_seconds = Some(3);
    Ok(conf)
}
