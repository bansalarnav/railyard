use std::{
    env,
    path::{Path, PathBuf},
};

pub(crate) fn runtime_dir() -> PathBuf {
    state_root().join("server")
}

pub(crate) fn database_path() -> PathBuf {
    runtime_dir().join("railyard.db")
}

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
