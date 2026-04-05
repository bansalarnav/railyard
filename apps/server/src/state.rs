use axum::http::{HeaderMap, header};
use std::{
    collections::BTreeMap,
    env,
    net::{IpAddr, SocketAddr},
    sync::Arc,
};

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) base_domain: String,
    pub(crate) proxy_addr: SocketAddr,
    pub(crate) axum_addr: SocketAddr,
    pub(crate) service_upstreams: Arc<BTreeMap<String, SocketAddr>>,
}

impl AppState {
    pub(crate) fn load() -> Self {
        Self {
            base_domain: base_domain(),
            proxy_addr: SocketAddr::from((proxy_host(), proxy_port())),
            axum_addr: SocketAddr::from((axum_host(), axum_port())),
            service_upstreams: Arc::new(configured_service_upstreams()),
        }
    }
}

pub(crate) fn display_url(host: &str, port: u16) -> String {
    if port == 80 {
        format!("http://{host}")
    } else {
        format!("http://{host}:{port}")
    }
}

pub(crate) fn requested_service(headers: &HeaderMap, base_domain: &str) -> Option<String> {
    headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .map(host_without_port)
        .and_then(|host| service_from_host(host, base_domain))
}

pub(crate) fn service_upstream_for_host(
    host: &str,
    base_domain: &str,
    service_upstreams: &BTreeMap<String, SocketAddr>,
) -> Option<(String, SocketAddr)> {
    let service = service_from_host(host_without_port(host), base_domain)?;

    if service == "admin" {
        return None;
    }

    service_upstreams
        .get(&service)
        .copied()
        .map(|addr| (service, addr))
}

fn configured_service_upstreams() -> BTreeMap<String, SocketAddr> {
    env::vars()
        .filter_map(|(key, value)| {
            let service = key.strip_prefix("CONTAINER_UPSTREAM_")?;
            let service = env_key_to_service_name(service);
            let upstream = value.parse().unwrap_or_else(|_| {
                panic!("invalid upstream socket address for {key}: {value:?}");
            });
            Some((service, upstream))
        })
        .collect()
}

fn proxy_host() -> IpAddr {
    match env::var("PROXY_HOST") {
        Ok(value) => value.parse().unwrap_or_else(|_| {
            panic!("PROXY_HOST must be a valid IP address, got {value:?}");
        }),
        Err(_) => IpAddr::from([0, 0, 0, 0]),
    }
}

fn proxy_port() -> u16 {
    match env::var("PROXY_PORT") {
        Ok(value) => value.parse().unwrap_or_else(|_| {
            panic!("PROXY_PORT must be a valid port number, got {value:?}");
        }),
        Err(_) => 8080,
    }
}

fn axum_host() -> IpAddr {
    match env::var("AXUM_HOST") {
        Ok(value) => value.parse().unwrap_or_else(|_| {
            panic!("AXUM_HOST must be a valid IP address, got {value:?}");
        }),
        Err(_) => IpAddr::from([127, 0, 0, 1]),
    }
}

fn axum_port() -> u16 {
    match env::var("AXUM_PORT") {
        Ok(value) => value.parse().unwrap_or_else(|_| {
            panic!("AXUM_PORT must be a valid port number, got {value:?}");
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

fn service_from_host(host: &str, base_domain: &str) -> Option<String> {
    let host = host.to_ascii_lowercase();

    subdomain_for_base(&host, "localhost").or_else(|| subdomain_for_base(&host, base_domain))
}

fn env_key_to_service_name(key: &str) -> String {
    key.to_ascii_lowercase().replace('_', "-")
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
