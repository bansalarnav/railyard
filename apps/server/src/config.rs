use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ServerConfig {
    pub(crate) public_base_url: String,
}

#[derive(Clone)]
pub(crate) struct ServerConfigStore {
    path: PathBuf,
}

impl ServerConfigStore {
    pub(crate) fn load() -> Self {
        Self {
            path: server_config_path(),
        }
    }

    pub(crate) fn read(&self) -> io::Result<Option<ServerConfig>> {
        if !self.path.exists() {
            return Ok(None);
        }

        let raw = fs::read_to_string(&self.path)?;
        let config = serde_json::from_str(&raw)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
        Ok(Some(config))
    }

    pub(crate) fn write(&self, config: &ServerConfig) -> io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }

        let raw = serde_json::to_string_pretty(config)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
        fs::write(&self.path, raw)
    }

    pub(crate) fn control_plane_url(&self) -> io::Result<String> {
        let config = self
            .read()?
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "server config is not initialized"))?;
        derive_control_plane_url(&config.public_base_url)
    }
}

fn server_config_path() -> PathBuf {
    config_root().join("server").join("config.json")
}

fn config_root() -> PathBuf {
    if let Ok(path) = env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(path).join("aethon");
    }

    let home = env::var("HOME").expect("HOME must be set when XDG_CONFIG_HOME is unset");
    Path::new(&home).join(".config").join("aethon")
}

fn derive_control_plane_url(public_base_url: &str) -> io::Result<String> {
    let mut url = url::Url::parse(public_base_url)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error.to_string()))?;

    match url.host_str() {
        Some(host) if host.parse::<std::net::IpAddr>().is_ok() => {
            let base_path = url.path().trim_end_matches('/');
            let next_path = if base_path.is_empty() || base_path == "/" {
                "/admin".to_string()
            } else {
                format!("{base_path}/admin")
            };
            url.set_path(&next_path);
        }
        Some(host) => {
            let admin_host = if host.starts_with("admin.") {
                host.to_string()
            } else {
                format!("admin.{host}")
            };
            url.set_host(Some(&admin_host))
                .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid admin host"))?;
            url.set_path("");
        }
        None => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "public base URL must include a host",
            ));
        }
    }

    Ok(url.to_string().trim_end_matches('/').to_string())
}
