use std::{
    collections::{BTreeMap, HashMap},
    env, io,
    net::{IpAddr, SocketAddr},
    str::FromStr,
    sync::{Arc, Mutex},
};

use crate::db::Db;

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) proxy_addr: SocketAddr,
    pub(crate) api_addr: SocketAddr,
    pub(crate) service_upstreams: Arc<BTreeMap<String, SocketAddr>>,
}
#[derive(Clone)]
pub(crate) struct ApiState {
    pub(crate) app: AppState,
    pub(crate) db: Arc<Db>,
    pub(crate) seen_nonces: Arc<Mutex<HashMap<String, u64>>>,
}

impl AppState {
    pub(crate) fn load() -> io::Result<Self> {
        let proxy_host: IpAddr = parsed_env(
            "RAILYARD_PROXY_HOST",
            [127, 0, 0, 1].into(),
            "an IP address",
        )?;
        let proxy_port: u16 = parsed_env("RAILYARD_PROXY_PORT", 3000, "a port number")?;
        let api_host: IpAddr =
            parsed_env("RAILYARD_API_HOST", [127, 0, 0, 1].into(), "an IP address")?;
        let api_port: u16 = parsed_env("RAILYARD_API_PORT", 3001, "a port number")?;

        Ok(Self {
            proxy_addr: SocketAddr::from((proxy_host, proxy_port)),
            api_addr: SocketAddr::from((api_host, api_port)),
            service_upstreams: Arc::new(configured_service_upstreams()?),
        })
    }
}

const UPSTREAM_ENV_PREFIX: &str = "RAILYARD_CONTAINER_UPSTREAM_";

fn configured_service_upstreams() -> io::Result<BTreeMap<String, SocketAddr>> {
    env::vars()
        .filter(|(key, _)| key.starts_with(UPSTREAM_ENV_PREFIX))
        .map(|(key, value)| {
            let service = env_key_to_service_name(&key[UPSTREAM_ENV_PREFIX.len()..]);
            let upstream = value
                .parse()
                .map_err(|_| invalid_env(&key, &value, "a socket address"))?;
            Ok((service, upstream))
        })
        .collect()
}

pub(crate) fn parsed_env<T: FromStr>(name: &str, default: T, expected: &str) -> io::Result<T> {
    match env::var(name) {
        Ok(value) => value
            .parse()
            .map_err(|_| invalid_env(name, &value, expected)),
        Err(_) => Ok(default),
    }
}

fn invalid_env(name: &str, value: &str, expected: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("{name} must be {expected}, got {value:?}"),
    )
}

fn env_key_to_service_name(key: &str) -> String {
    key.to_ascii_lowercase().replace('_', "-")
}
