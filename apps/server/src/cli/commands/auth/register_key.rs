use std::error::Error;

use crate::auth::{AuthStore, RegisterKeyResponse};
use crate::config::ServerConfigStore;

pub(crate) fn run(name: String, public_key: String) -> Result<(), Box<dyn Error>> {
    // Resolve the URL first so a missing config does not leave an orphan key.
    let server_url = ServerConfigStore::load().control_plane_url()?;
    let record = AuthStore::load().register_key(name, public_key, vec!["admin".to_string()])?;

    let response = RegisterKeyResponse {
        key_id: record.key_id,
        device_name: record.device_name,
        server_url,
    };

    println!("{}", serde_json::to_string(&response)?);
    Ok(())
}
