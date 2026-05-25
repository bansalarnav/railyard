use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use std::{fs, path::Path, thread, time::Duration};

use super::server::run_server;

const PID_FILE_PATH: &str = "/tmp/pingora.pid";

pub(crate) fn up() {
    if let Some(pid) = read_running_pid() {
        println!("Railyard server is already running with pid {pid}");
        return;
    }

    run_server(true);
}

pub(crate) fn down() {
    let pid_path = pid_file_path();
    let Some(pid) = read_pid_file(&pid_path) else {
        println!("Railyard server is not running");
        return;
    };

    if !process_exists(pid) {
        let _ = fs::remove_file(&pid_path);
        println!("Removed stale pid file for pid {pid}");
        return;
    }

    kill(Pid::from_raw(pid), Signal::SIGTERM).expect("failed to signal running server");

    for _ in 0..50 {
        if !process_exists(pid) {
            let _ = fs::remove_file(&pid_path);
            println!("Stopped Railyard server (pid {pid})");
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }

    println!("Sent SIGTERM to pid {pid}, but it is still shutting down");
}

fn read_running_pid() -> Option<i32> {
    let pid = read_pid_file(&pid_file_path())?;
    if process_exists(pid) {
        Some(pid)
    } else {
        let _ = fs::remove_file(pid_file_path());
        None
    }
}

fn pid_file_path() -> &'static Path {
    Path::new(PID_FILE_PATH)
}

fn read_pid_file(path: &Path) -> Option<i32> {
    let raw = fs::read_to_string(path).ok()?;
    raw.trim().parse().ok()
}

fn process_exists(pid: i32) -> bool {
    kill(Pid::from_raw(pid), None).is_ok()
}
