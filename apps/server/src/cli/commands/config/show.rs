use crate::config::ServerConfigStore;

pub(crate) fn run() {
    let store = ServerConfigStore::load();
    let config = store
        .read()
        .expect("failed to read server config")
        .expect("server config is not initialized");
    let control_plane_url = store
        .control_plane_url()
        .expect("failed to resolve control plane URL");

    println!(
        "{}",
        serde_json::json!({
            "public_base_url": config.public_base_url,
            "control_plane_url": control_plane_url,
        })
    );
}
