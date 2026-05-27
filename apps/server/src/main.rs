mod app;
mod auth;
mod cli;
mod config;
mod daemon;

fn main() {
    env_logger::init();
    cli::run();
}
