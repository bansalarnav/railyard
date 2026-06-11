use std::io;

use crate::config::{ServerConfig, ServerConfigStore};

pub(crate) fn run(public_url: String) -> io::Result<()> {
    ServerConfigStore::load().write(&ServerConfig {
        public_base_url: public_url,
    })
}
