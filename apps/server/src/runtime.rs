use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use pingora::proxy::http_proxy_service;
use pingora::server::Server;
use pingora::server::configuration::Opt;
use pingora::services::background::background_service;
use std::{
    fs,
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

use crate::control_plane::AxumControlPlane;
use crate::proxy::{ControlPlaneProxy, RoutingTable};
use crate::state::{AppState, display_url};

const RUNTIME_DIR: &str = "/tmp/aethon-server";
const PINGORA_CONF_PATH: &str = "/tmp/aethon-server/pingora.yaml";
const PID_FILE_PATH: &str = "/tmp/aethon-server/server.pid";
const ERROR_LOG_PATH: &str = "/tmp/aethon-server/error.log";
const UPGRADE_SOCK_PATH: &str = "/tmp/aethon-server/upgrade.sock";

pub(crate) fn up() {
    ensure_runtime_layout().expect("failed to create runtime directory");

    if let Some(pid) = read_running_pid() {
        println!("Aethon server is already running with pid {pid}");
        return;
    }

    write_pingora_conf(runtime_conf_path()).expect("failed to write pingora config");
    run_server(true);
}

pub(crate) fn down() {
    let pid_path = runtime_pid_path();
    let Some(pid) = read_pid_file(&pid_path) else {
        println!("Aethon server is not running");
        return;
    };

    if !process_exists(pid) {
        let _ = fs::remove_file(&pid_path);
        println!("Removed stale pid file for pid {pid}");
        return;
    }

    kill(Pid::from_raw(pid), Signal::SIGTERM).expect("failed to signal running server");

    for _ in 0..50 {
        if !process_exists(pid) {
            let _ = fs::remove_file(&pid_path);
            println!("Stopped Aethon server (pid {pid})");
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }

    println!("Sent SIGTERM to pid {pid}, but it is still shutting down");
}

fn run_server(daemon: bool) {
    let config = config::AppConfig::default();
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
        config.app_name, proxy_addr
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

fn runtime_dir() -> &'static Path {
    Path::new(RUNTIME_DIR)
}

fn runtime_conf_path() -> PathBuf {
    PathBuf::from(PINGORA_CONF_PATH)
}

fn runtime_pid_path() -> PathBuf {
    PathBuf::from(PID_FILE_PATH)
}

fn ensure_runtime_layout() -> std::io::Result<()> {
    fs::create_dir_all(runtime_dir())
}

fn write_pingora_conf(path: PathBuf) -> std::io::Result<()> {
    let contents = format!(
        concat!(
            "---\n",
            "version: 1\n",
            "daemon: true\n",
            "pid_file: {pid}\n",
            "upgrade_sock: {upgrade_sock}\n",
            "error_log: {error_log}\n",
            "threads: 1\n"
        ),
        pid = PID_FILE_PATH,
        upgrade_sock = UPGRADE_SOCK_PATH,
        error_log = ERROR_LOG_PATH,
    );

    fs::write(path, contents)
}

fn read_running_pid() -> Option<i32> {
    let pid = read_pid_file(&runtime_pid_path())?;
    if process_exists(pid) {
        Some(pid)
    } else {
        let _ = fs::remove_file(runtime_pid_path());
        None
    }
}

fn read_pid_file(path: &Path) -> Option<i32> {
    let raw = fs::read_to_string(path).ok()?;
    raw.trim().parse().ok()
}

fn process_exists(pid: i32) -> bool {
    kill(Pid::from_raw(pid), None).is_ok()
}
