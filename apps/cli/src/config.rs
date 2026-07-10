use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use ed25519_dalek::SigningKey;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ClientProfile {
    pub(crate) server_url: String,
    pub(crate) key_id: String,
    pub(crate) private_key_path: String,
}

#[derive(Serialize, Deserialize)]
struct StoredPrivateKey {
    key_id: String,
    secret_key_base64: String,
}

pub(crate) fn write_profile(profile_name: &str, profile: &ClientProfile) -> io::Result<PathBuf> {
    let path = profile_path(profile_name)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let raw = serde_json::to_string_pretty(profile).map_err(invalid_data)?;
    fs::write(&path, raw)?;
    Ok(path)
}

pub(crate) fn read_profile(profile_name: &str) -> io::Result<ClientProfile> {
    let raw = fs::read_to_string(profile_path(profile_name)?)?;
    serde_json::from_str(&raw).map_err(invalid_data)
}

pub(crate) fn list_profiles() -> io::Result<Vec<(String, ClientProfile)>> {
    let dir = config_root()?.join("client").join("profiles");
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };

    let mut profiles = Vec::new();
    for entry in entries {
        let path = entry?.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let Some(name) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };

        match serde_json::from_str(&fs::read_to_string(&path)?) {
            Ok(profile) => profiles.push((name.to_string(), profile)),
            Err(error) => eprintln!("warning: skipping unreadable profile {name}: {error}"),
        }
    }

    profiles.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(profiles)
}

/// Global client state that is not per-profile: the `projects` map records
/// which profile each known project id was created or linked through, so
/// later project commands pin to the same server without flags.
#[derive(Debug, Default, Serialize, Deserialize)]
struct GlobalConfig {
    #[serde(default)]
    projects: BTreeMap<String, String>,
}

pub(crate) fn record_project_binding(project_id: &str, profile_name: &str) -> io::Result<()> {
    let path = config_root()?.join("client").join("config.json");
    let mut config: GlobalConfig = match fs::read_to_string(&path) {
        Ok(raw) => serde_json::from_str(&raw).map_err(invalid_data)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => GlobalConfig::default(),
        Err(error) => return Err(error),
    };

    config
        .projects
        .insert(project_id.to_string(), profile_name.to_string());

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(
        &path,
        serde_json::to_string_pretty(&config).map_err(invalid_data)?,
    )
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

pub(crate) fn sanitize_profile_name(raw: &str) -> String {
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

fn profile_path(profile_name: &str) -> io::Result<PathBuf> {
    Ok(config_root()?
        .join("client")
        .join("profiles")
        .join(format!("{profile_name}.json")))
}

fn key_path(key_id: &str) -> io::Result<PathBuf> {
    Ok(config_root()?
        .join("client")
        .join("keys")
        .join(format!("{key_id}.json")))
}

fn config_root() -> io::Result<PathBuf> {
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
