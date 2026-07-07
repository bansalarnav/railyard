mod cli;
mod db;
mod http;
mod paths;

fn main() {
    env_logger::init();

    if let Err(error) = cli::run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
