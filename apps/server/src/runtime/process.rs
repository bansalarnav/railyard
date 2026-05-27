use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use std::{
    env, fs, io,
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

use super::server::run_server;

pub(crate) fn up() {
    if let Some(pid) = read_running_pid() {
        println!("Railyard server is already running with pid {pid}");
        return;
    }

    ensure_runtime_dir().expect("failed to create railyard runtime directory");
    run_server(true, &pid_file_path(), &upgrade_sock_path());
}

pub(crate) fn serve() {
    ensure_runtime_dir().expect("failed to create railyard runtime directory");
    run_server(false, &pid_file_path(), &upgrade_sock_path());
}

pub(crate) fn down() {
    if stop_running_server() {
        return;
    }
}

pub(crate) fn restart() {
    stop_running_server();
    up();
}

pub(crate) fn status() {
    match read_running_pid() {
        Some(pid) => println!("Railyard server is running with pid {pid}"),
        None => println!("Railyard server is not running"),
    }
}

fn stop_running_server() -> bool {
    let pid_path = pid_file_path();
    let Some(pid) = read_pid_file(&pid_path) else {
        println!("Railyard server is not running");
        return false;
    };

    if !process_exists(pid) {
        let _ = fs::remove_file(&pid_path);
        println!("Removed stale pid file for pid {pid}");
        return false;
    }

    kill(Pid::from_raw(pid), Signal::SIGTERM).expect("failed to signal running server");

    for _ in 0..50 {
        if !process_exists(pid) {
            let _ = fs::remove_file(&pid_path);
            println!("Stopped Railyard server (pid {pid})");
            return true;
        }
        thread::sleep(Duration::from_millis(100));
    }

    println!("Sent SIGTERM to pid {pid}, but it is still shutting down");
    false
}

fn read_running_pid() -> Option<i32> {
    let path = pid_file_path();
    let pid = read_pid_file(&path)?;
    if process_exists(pid) {
        Some(pid)
    } else {
        let _ = fs::remove_file(path);
        None
    }
}

fn ensure_runtime_dir() -> io::Result<()> {
    fs::create_dir_all(runtime_dir())
}

fn pid_file_path() -> PathBuf {
    runtime_dir().join("server.pid")
}

fn upgrade_sock_path() -> PathBuf {
    runtime_dir().join("upgrade.sock")
}

fn runtime_dir() -> PathBuf {
    state_root().join("server")
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

fn read_pid_file(path: &Path) -> Option<i32> {
    let raw = fs::read_to_string(path).ok()?;
    raw.trim().parse().ok()
}

fn process_exists(pid: i32) -> bool {
    kill(Pid::from_raw(pid), None).is_ok()
}
