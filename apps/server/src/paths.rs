use std::{
    env,
    path::{Path, PathBuf},
};

fn state_root() -> PathBuf {
    if let Ok(path) = env::var("XDG_STATE_HOME") {
        return PathBuf::from(path).join("railyard");
    }

    let home = env::var("HOME").expect("HOME must be set when XDG_STATE_HOME is unset");
    Path::new(&home)
        .join(".local")
        .join("state")
        .join("railyard")
}

pub(crate) fn runtime_dir() -> PathBuf {
    env::var("RAILYARD_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| default_server_dir())
}

fn data_dir() -> PathBuf {
    env::var("RAILYARD_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| default_server_dir())
}

fn default_server_dir() -> PathBuf {
    state_root().join("server")
}

pub(crate) fn database_path() -> PathBuf {
    data_dir().join("railyard.db")
}

/// Uploaded archives and their unpacked trees, one directory per release:
/// releases/<project_id>/<release_id>/{archive.tar.gz, source/}.
pub(crate) fn release_dir(project_id: &str, release_id: &str) -> PathBuf {
    data_dir()
        .join("releases")
        .join(project_id)
        .join(release_id)
}

/// Local admin API: requests on this socket are trusted as a server admin,
/// gated by file permissions instead of request signatures.
pub(crate) fn admin_sock_path() -> PathBuf {
    runtime_dir().join("admin.sock")
}
