mod cli;
mod control_plane;
mod proxy;
mod runtime;
mod state;

fn main() {
    env_logger::init();
    cli::run();
}
