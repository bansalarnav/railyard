mod api;
mod cli;
mod config;
mod paths;
mod proxy;
mod server;

fn main() {
    env_logger::init();

    if let Err(error) = cli::run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
