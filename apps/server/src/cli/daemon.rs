use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use std::{
    fs, io,
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

use crate::http::run_server;
use crate::paths::runtime_dir;

pub(crate) fn up(foreground: bool) -> io::Result<()> {
    if let Some(pid) = read_running_pid() {
        println!("Railyard server is already running with pid {pid}");
        return Ok(());
    }

    ensure_runtime_dir()?;
    run_server(!foreground, &pid_file_path(), &upgrade_sock_path())
}

pub(crate) fn down() -> io::Result<()> {
    stop_running_server()
}

pub(crate) fn restart() -> io::Result<()> {
    stop_running_server()?;
    up(false)
}

pub(crate) fn status() {
    match read_running_pid() {
        Some(pid) => println!("Railyard server is running with pid {pid}"),
        None => println!("Railyard server is not running"),
    }
}

fn stop_running_server() -> io::Result<()> {
    let pid_path = pid_file_path();
    let Some(pid) = read_pid_file(&pid_path) else {
        println!("Railyard server is not running");
        return Ok(());
    };

    if !process_exists(pid) {
        let _ = fs::remove_file(&pid_path);
        println!("Removed stale pid file for pid {pid}");
        return Ok(());
    }

    kill(Pid::from_raw(pid), Signal::SIGTERM)
        .map_err(|errno| io::Error::from_raw_os_error(errno as i32))?;

    for _ in 0..50 {
        if !process_exists(pid) {
            let _ = fs::remove_file(&pid_path);
            println!("Stopped Railyard server (pid {pid})");
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }

    println!("Sent SIGTERM to pid {pid}, but it is still shutting down");
    Ok(())
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

fn read_pid_file(path: &Path) -> Option<i32> {
    let raw = fs::read_to_string(path).ok()?;
    raw.trim().parse().ok()
}

fn process_exists(pid: i32) -> bool {
    kill(Pid::from_raw(pid), None).is_ok()
}
