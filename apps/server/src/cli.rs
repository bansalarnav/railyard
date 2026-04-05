use clap::{Parser, Subcommand};

use crate::runtime;

#[derive(Parser)]
#[command(name = "aethon-server")]
#[command(about = "Aethon control-plane and ingress server")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Up,
    Down,
}

pub fn run() {
    let cli = Cli::parse();

    match cli.command {
        Command::Up => runtime::up(),
        Command::Down => runtime::down(),
    }
}
