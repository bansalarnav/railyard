use std::error::Error;

use crate::config::ServerConfigStore;

pub(crate) fn run() -> Result<(), Box<dyn Error>> {
    let store = ServerConfigStore::load();
    let config = store.read()?.ok_or("server config is not initialized")?;
    let control_plane_url = store.control_plane_url()?;

    println!(
        "{}",
        serde_json::json!({
            "public_base_url": config.public_base_url,
            "control_plane_url": control_plane_url,
        })
    );

    Ok(())
}
