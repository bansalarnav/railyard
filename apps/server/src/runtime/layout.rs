use std::{
    fs,
    path::{Path, PathBuf},
};

const RUNTIME_DIR: &str = "/tmp/aethon-server";
const PINGORA_CONF_PATH: &str = "/tmp/aethon-server/pingora.yaml";
const PID_FILE_PATH: &str = "/tmp/aethon-server/server.pid";
const ERROR_LOG_PATH: &str = "/tmp/aethon-server/error.log";
const UPGRADE_SOCK_PATH: &str = "/tmp/aethon-server/upgrade.sock";

pub(super) fn runtime_conf_path() -> PathBuf {
    PathBuf::from(PINGORA_CONF_PATH)
}

pub(super) fn runtime_pid_path() -> PathBuf {
    PathBuf::from(PID_FILE_PATH)
}

pub(super) fn ensure_runtime_layout() -> std::io::Result<()> {
    fs::create_dir_all(Path::new(RUNTIME_DIR))
}

pub(super) fn write_pingora_conf(path: PathBuf) -> std::io::Result<()> {
    let contents = format!(
        concat!(
            "---\n",
            "version: 1\n",
            "daemon: true\n",
            "pid_file: {pid}\n",
            "upgrade_sock: {upgrade_sock}\n",
            "error_log: {error_log}\n",
            "threads: 1\n"
        ),
        pid = PID_FILE_PATH,
        upgrade_sock = UPGRADE_SOCK_PATH,
        error_log = ERROR_LOG_PATH,
    );

    fs::write(path, contents)
}
