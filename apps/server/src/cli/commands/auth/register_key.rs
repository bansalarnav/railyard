use crate::auth::{AuthStore, RegisterKeyResponse};
use crate::config::ServerConfigStore;

pub(crate) fn run(name: String, public_key: String) {
    let config_store = ServerConfigStore::load();
    let record = AuthStore::load()
        .register_key(name, public_key, vec!["admin".to_string()])
        .expect("failed to register auth key");
    let server_url = config_store
        .control_plane_url()
        .expect("failed to resolve control plane URL");

    let response = RegisterKeyResponse {
        key_id: record.key_id,
        device_name: record.device_name,
        server_url,
    };

    println!(
        "{}",
        serde_json::to_string(&response).expect("failed to serialize register-key response")
    );
}
