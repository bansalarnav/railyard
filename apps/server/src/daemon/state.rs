use std::{
    collections::BTreeMap,
    env,
    net::{IpAddr, SocketAddr},
    sync::Arc,
};

use crate::auth::{AuthStore, NonceCache};

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) proxy_addr: SocketAddr,
    pub(crate) api_addr: SocketAddr,
    pub(crate) service_upstreams: Arc<BTreeMap<String, SocketAddr>>,
    pub(crate) auth_store: AuthStore,
    pub(crate) auth_nonce_cache: NonceCache,
}

impl AppState {
    pub(crate) fn load() -> Self {
        Self {
            proxy_addr: SocketAddr::from((proxy_host(), proxy_port())),
            api_addr: SocketAddr::from((api_host(), api_port())),
            service_upstreams: Arc::new(configured_service_upstreams()),
            auth_store: AuthStore::load(),
            auth_nonce_cache: NonceCache::default(),
        }
    }
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
        Err(_) => IpAddr::from([127, 0, 0, 1]),
    }
}

fn proxy_port() -> u16 {
    match env::var("PROXY_PORT") {
        Ok(value) => value.parse().unwrap_or_else(|_| {
            panic!("PROXY_PORT must be a valid port number, got {value:?}");
        }),
        Err(_) => 3000,
    }
}

fn api_host() -> IpAddr {
    match env::var("API_HOST") {
        Ok(value) => value.parse().unwrap_or_else(|_| {
            panic!("API_HOST must be a valid IP address, got {value:?}");
        }),
        Err(_) => IpAddr::from([127, 0, 0, 1]),
    }
}

fn api_port() -> u16 {
    match env::var("API_PORT") {
        Ok(value) => value.parse().unwrap_or_else(|_| {
            panic!("API_PORT must be a valid port number, got {value:?}");
        }),
        Err(_) => 3001,
    }
}

fn env_key_to_service_name(key: &str) -> String {
    key.to_ascii_lowercase().replace('_', "-")
}
