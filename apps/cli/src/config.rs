use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use ed25519_dalek::SigningKey;
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ClientProfile {
    pub(crate) server_url: String,
    pub(crate) ssh_target: String,
    pub(crate) key_id: String,
    pub(crate) device_name: String,
    pub(crate) private_key_path: String,
}

#[derive(Serialize, Deserialize)]
struct StoredPrivateKey {
    key_id: String,
    secret_key_base64: String,
}

pub(crate) fn write_profile(profile_name: &str, profile: &ClientProfile) -> io::Result<PathBuf> {
    let path = profile_path(profile_name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let raw = serde_json::to_string_pretty(profile).map_err(invalid_input)?;
    fs::write(&path, raw)?;
    Ok(path)
}

pub(crate) fn read_profile(profile_name: &str) -> io::Result<ClientProfile> {
    let raw = fs::read_to_string(profile_path(profile_name))?;
    serde_json::from_str(&raw).map_err(invalid_input)
}

pub(crate) fn write_signing_key(key_id: &str, signing_key: &SigningKey) -> io::Result<PathBuf> {
    let path = key_path(key_id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let raw = serde_json::to_string_pretty(&StoredPrivateKey {
        key_id: key_id.to_string(),
        secret_key_base64: BASE64_STANDARD.encode(signing_key.to_bytes()),
    })
    .map_err(invalid_input)?;

    fs::write(&path, raw)?;
    set_private_permissions(&path)?;
    Ok(path)
}

pub(crate) fn read_signing_key(path: &str) -> io::Result<SigningKey> {
    let raw = fs::read_to_string(path)?;
    let stored: StoredPrivateKey = serde_json::from_str(&raw).map_err(invalid_input)?;
    let secret_key = BASE64_STANDARD
        .decode(stored.secret_key_base64.as_bytes())
        .map_err(invalid_input)?;

    let secret_key: [u8; 32] = secret_key
        .try_into()
        .map_err(|_| invalid_input("secret key must be 32 bytes"))?;

    Ok(SigningKey::from_bytes(&secret_key))
}

pub(crate) fn default_device_name() -> String {
    std::process::Command::new("hostname")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|output| output.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "aethon-device".to_string())
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

fn profile_path(profile_name: &str) -> PathBuf {
    config_root()
        .join("client")
        .join("profiles")
        .join(format!("{profile_name}.json"))
}

fn key_path(key_id: &str) -> PathBuf {
    config_root()
        .join("client")
        .join("keys")
        .join(format!("{key_id}.json"))
}

fn config_root() -> PathBuf {
    if let Ok(path) = env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(path).join("aethon");
    }

    let home = env::var("HOME").expect("HOME must be set when XDG_CONFIG_HOME is unset");
    Path::new(&home).join(".config").join("aethon")
}

fn invalid_input(error: impl std::fmt::Display) -> io::Error {
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
