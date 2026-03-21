use axum::extract::State;
use axum::http::{HeaderMap, header};
use axum::{Router, routing::get};
use std::{
    env,
    net::{IpAddr, SocketAddr},
};

#[derive(Clone)]
struct AppState {
    base_domain: String,
}

#[tokio::main]
async fn main() {
    let config = config::AppConfig::default();
    let base_domain = base_domain();
    let app = Router::new().route("/", get(root)).with_state(AppState {
        base_domain: base_domain.clone(),
    });
    let addr = SocketAddr::from((server_host(), server_port()));

    println!("Starting {} server on http://{}", config.app_name, addr);
    println!("Dashboard URL: http://{}:{}", base_domain, addr.port());
    println!(
        "Example deployment URL: http://howdy.{}:{}",
        base_domain,
        addr.port()
    );

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind TCP listener");

    axum::serve(listener, app)
        .await
        .expect("server exited with error");
}

async fn root(State(state): State<AppState>, headers: HeaderMap) -> &'static str {
    if requested_service(&headers, &state.base_domain).as_deref() == Some("howdy") {
        "Howdy World"
    } else {
        "Hello World"
    }
}

fn server_host() -> IpAddr {
    match env::var("SERVER_HOST") {
        Ok(value) => value.parse().unwrap_or_else(|_| {
            panic!("SERVER_HOST must be a valid IP address, got {value:?}");
        }),
        Err(_) => IpAddr::from([127, 0, 0, 1]),
    }
}

fn server_port() -> u16 {
    match env::var("SERVER_PORT") {
        Ok(value) => value.parse().unwrap_or_else(|_| {
            panic!("SERVER_PORT must be a valid port number, got {value:?}");
        }),
        Err(_) => 3000,
    }
}

fn base_domain() -> String {
    match env::var("BASE_DOMAIN") {
        Ok(value) => value.trim().trim_end_matches('.').to_ascii_lowercase(),
        Err(_) => "127.0.0.1.nip.io".to_string(),
    }
}

fn requested_service(headers: &HeaderMap, base_domain: &str) -> Option<String> {
    headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .map(host_without_port)
        .and_then(|host| service_from_host(host, base_domain))
}

fn service_from_host(host: &str, base_domain: &str) -> Option<String> {
    let host = host.to_ascii_lowercase();

    subdomain_for_base(&host, "localhost").or_else(|| subdomain_for_base(&host, base_domain))
}

fn subdomain_for_base(host: &str, base: &str) -> Option<String> {
    if host == base {
        return None;
    }

    let prefix = host.strip_suffix(base)?;
    prefix.strip_suffix('.').map(ToOwned::to_owned)
}

fn host_without_port(host: &str) -> &str {
    if let Some((name, port)) = host.rsplit_once(':') {
        if port.chars().all(|char| char.is_ascii_digit()) {
            return name.trim_matches(&['[', ']'][..]);
        }
    }

    host.trim_matches(&['[', ']'][..])
}
