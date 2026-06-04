mod app;
mod auth;
mod cli;
mod config;
mod daemon;
mod http;

fn main() {
    env_logger::init();
    cli::run();
}
