use crate::config::{ServerConfig, ServerConfigStore};

pub(crate) fn run(public_url: String) {
    ServerConfigStore::load()
        .write(&ServerConfig {
            public_base_url: public_url,
        })
        .expect("failed to write server config");
}
