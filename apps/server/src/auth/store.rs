use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use ed25519_dalek::VerifyingKey;
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use ulid::Ulid;

#[derive(Clone)]
pub(crate) struct AuthStore {
    path: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct AuthKeyRecord {
    pub(crate) key_id: String,
    pub(crate) device_name: String,
    pub(crate) public_key_base64: String,
    pub(crate) scopes: Vec<String>,
    pub(crate) created_at: u64,
    pub(crate) revoked_at: Option<u64>,
}

#[derive(Debug, Serialize)]
pub(crate) struct RegisterKeyResponse {
    pub(crate) key_id: String,
    pub(crate) device_name: String,
    pub(crate) server_url: String,
}

#[derive(Debug, Serialize, Deserialize, Default)]
struct AuthStoreFile {
    keys: Vec<AuthKeyRecord>,
}

impl AuthStore {
    pub(crate) fn load() -> Self {
        Self {
            path: server_auth_store_path(),
        }
    }

    pub(crate) fn register_key(
        &self,
        device_name: String,
        public_key_base64: String,
        scopes: Vec<String>,
    ) -> io::Result<AuthKeyRecord> {
        let mut store = self.read_file()?;
        let record = AuthKeyRecord {
            key_id: format!("dev_{}", Ulid::new()),
            device_name,
            public_key_base64,
            scopes,
            created_at: unix_timestamp(),
            revoked_at: None,
        };

        store.keys.push(record.clone());
        self.write_file(&store)?;
        Ok(record)
    }

    pub(crate) fn list_keys(&self) -> io::Result<Vec<AuthKeyRecord>> {
        Ok(self.read_file()?.keys)
    }

    pub(crate) fn revoke_key(&self, key_id: &str) -> io::Result<Option<AuthKeyRecord>> {
        let mut store = self.read_file()?;
        let now = unix_timestamp();

        let maybe_record = store
            .keys
            .iter_mut()
            .find(|record| record.key_id == key_id && record.revoked_at.is_none());

        let Some(record) = maybe_record else {
            return Ok(None);
        };

        record.revoked_at = Some(now);
        let record = record.clone();
        self.write_file(&store)?;
        Ok(Some(record))
    }

    pub(crate) fn find_active_key(&self, key_id: &str) -> io::Result<Option<AuthKeyRecord>> {
        let record = self
            .read_file()?
            .keys
            .into_iter()
            .find(|record| record.key_id == key_id && record.revoked_at.is_none());
        Ok(record)
    }

    pub(crate) fn verifying_key_for(&self, key_id: &str) -> io::Result<Option<VerifyingKey>> {
        let Some(record) = self.find_active_key(key_id)? else {
            return Ok(None);
        };

        let key_bytes = BASE64_STANDARD
            .decode(record.public_key_base64.as_bytes())
            .map_err(invalid_input)?;

        let verifying_key = VerifyingKey::from_bytes(
            &key_bytes
                .try_into()
                .map_err(|_| invalid_input("public key must be 32 bytes"))?,
        )
        .map_err(invalid_input)?;

        Ok(Some(verifying_key))
    }

    fn read_file(&self) -> io::Result<AuthStoreFile> {
        if !self.path.exists() {
            return Ok(AuthStoreFile::default());
        }

        let raw = fs::read_to_string(&self.path)?;
        serde_json::from_str(&raw).map_err(invalid_input)
    }

    fn write_file(&self, store: &AuthStoreFile) -> io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }

        let raw = serde_json::to_string_pretty(store).map_err(invalid_input)?;
        fs::write(&self.path, raw)
    }
}

fn server_auth_store_path() -> PathBuf {
    config_root().join("server").join("auth-keys.json")
}

fn config_root() -> PathBuf {
    if let Ok(path) = env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(path).join("aethon");
    }

    let home = env::var("HOME").expect("HOME must be set when XDG_CONFIG_HOME is unset");
    Path::new(&home).join(".config").join("aethon")
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_secs()
}

fn invalid_input(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}
