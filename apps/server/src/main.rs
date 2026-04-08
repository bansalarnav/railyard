mod auth;
mod api;
mod app;
mod cli;
mod config;
mod proxy;
mod runtime;
mod state;

fn main() {
    env_logger::init();
    cli::run();
}
