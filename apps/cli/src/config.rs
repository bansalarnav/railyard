use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use ed25519_dalek::SigningKey;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// One identity on one server: where it lives and the key that proves who
/// we are to it. Stored at `client/servers/<name>.json`.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ServerConfig {
    pub(crate) server_url: String,
    pub(crate) key_id: String,
    pub(crate) private_key_path: String,
}

#[derive(Serialize, Deserialize)]
struct StoredPrivateKey {
    key_id: String,
    secret_key_base64: String,
}

pub(crate) fn write_server(server_name: &str, server: &ServerConfig) -> io::Result<PathBuf> {
    let path = server_path(server_name)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let raw = serde_json::to_string_pretty(server).map_err(invalid_data)?;
    fs::write(&path, raw)?;
    Ok(path)
}

pub(crate) fn read_server(server_name: &str) -> io::Result<ServerConfig> {
    let raw = fs::read_to_string(server_path(server_name)?)?;
    serde_json::from_str(&raw).map_err(invalid_data)
}

/// Delete a server entry and its private key file.
pub(crate) fn remove_server(server_name: &str) -> io::Result<()> {
    if let Ok(server) = read_server(server_name) {
        let _ = fs::remove_file(&server.private_key_path);
    }
    fs::remove_file(server_path(server_name)?)
}

pub(crate) fn list_servers() -> io::Result<Vec<(String, ServerConfig)>> {
    let dir = config_root()?.join("client").join("servers");
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };

    let mut servers = Vec::new();
    for entry in entries {
        let path = entry?.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let Some(name) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };

        match serde_json::from_str(&fs::read_to_string(&path)?) {
            Ok(server) => servers.push((name.to_string(), server)),
            Err(error) => eprintln!("warning: skipping unreadable server {name}: {error}"),
        }
    }

    servers.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(servers)
}

/// Global client state that is not per-server: the `projects` map records
/// which server each known project id was created or linked through, so
/// later project commands pin to the same server without flags.
#[derive(Debug, Default, Serialize, Deserialize)]
struct GlobalConfig {
    #[serde(default)]
    projects: BTreeMap<String, String>,
}

pub(crate) fn read_project_binding(project_id: &str) -> io::Result<Option<String>> {
    Ok(read_global_config()?.projects.get(project_id).cloned())
}

pub(crate) fn record_project_binding(project_id: &str, server_name: &str) -> io::Result<()> {
    let mut config = read_global_config()?;
    config
        .projects
        .insert(project_id.to_string(), server_name.to_string());
    write_global_config(&config)
}

/// Forget which server a project deploys through; returns the server name
/// the binding pointed at, if there was one.
pub(crate) fn remove_project_binding(project_id: &str) -> io::Result<Option<String>> {
    let mut config = read_global_config()?;
    let removed = config.projects.remove(project_id);
    if removed.is_some() {
        write_global_config(&config)?;
    }
    Ok(removed)
}

/// Point every project bound to server `from` at `to`; returns how many moved.
pub(crate) fn rebind_projects(from: &str, to: &str) -> io::Result<usize> {
    let mut config = read_global_config()?;
    let mut moved = 0;
    for server in config.projects.values_mut() {
        if server.as_str() == from {
            *server = to.to_string();
            moved += 1;
        }
    }
    if moved > 0 {
        write_global_config(&config)?;
    }
    Ok(moved)
}

fn write_global_config(config: &GlobalConfig) -> io::Result<()> {
    let path = global_config_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(
        &path,
        serde_json::to_string_pretty(config).map_err(invalid_data)?,
    )
}

fn read_global_config() -> io::Result<GlobalConfig> {
    match fs::read_to_string(global_config_path()?) {
        Ok(raw) => serde_json::from_str(&raw).map_err(invalid_data),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(GlobalConfig::default()),
        Err(error) => Err(error),
    }
}

fn global_config_path() -> io::Result<PathBuf> {
    Ok(config_root()?.join("client").join("config.json"))
}

pub(crate) fn write_signing_key(key_id: &str, signing_key: &SigningKey) -> io::Result<PathBuf> {
    let path = key_path(key_id)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let raw = serde_json::to_string_pretty(&StoredPrivateKey {
        key_id: key_id.to_string(),
        secret_key_base64: BASE64_STANDARD.encode(signing_key.to_bytes()),
    })
    .map_err(invalid_data)?;

    fs::write(&path, raw)?;
    set_private_permissions(&path)?;
    Ok(path)
}

pub(crate) fn read_signing_key(path: &str) -> io::Result<SigningKey> {
    let raw = fs::read_to_string(path)?;
    let stored: StoredPrivateKey = serde_json::from_str(&raw).map_err(invalid_data)?;
    let secret_key = BASE64_STANDARD
        .decode(stored.secret_key_base64.as_bytes())
        .map_err(invalid_data)?;

    let secret_key: [u8; 32] = secret_key
        .try_into()
        .map_err(|_| invalid_data("secret key must be 32 bytes"))?;

    Ok(SigningKey::from_bytes(&secret_key))
}

pub(crate) fn sanitize_server_name(raw: &str) -> String {
    let sanitized: String = raw
        .chars()
        .map(|char| {
            if char.is_ascii_alphanumeric() || matches!(char, '-' | '_') {
                char
            } else {
                '-'
            }
        })
        .collect();

    sanitized.trim_matches('-').to_string()
}

fn server_path(server_name: &str) -> io::Result<PathBuf> {
    Ok(config_root()?
        .join("client")
        .join("servers")
        .join(format!("{server_name}.json")))
}

fn key_path(key_id: &str) -> io::Result<PathBuf> {
    Ok(config_root()?
        .join("client")
        .join("keys")
        .join(format!("{key_id}.json")))
}

fn config_root() -> io::Result<PathBuf> {
    if env::var_os("RAILYARD_DEV").is_some() {
        return Ok(PathBuf::from(".dev-state"));
    }

    if let Some(path) = env::var_os("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(path).join("railyard"));
    }

    if let Some(home) = env::var_os("HOME") {
        return Ok(Path::new(&home).join(".config").join("railyard"));
    }
    if let Some(appdata) = env::var_os("APPDATA") {
        return Ok(PathBuf::from(appdata).join("railyard"));
    }

    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "could not locate a config directory: set XDG_CONFIG_HOME, HOME, or APPDATA",
    ))
}

fn invalid_data(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

#[cfg(unix)]
fn set_private_permissions(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn set_private_permissions(_path: &Path) -> io::Result<()> {
    Ok(())
}
