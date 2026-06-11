mod app;
mod auth;
mod cli;
mod config;
mod daemon;
mod http;

fn main() {
    env_logger::init();

    if let Err(error) = cli::run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
