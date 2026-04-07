mod commands;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "aethon-server")]
#[command(about = "Aethon server daemon control CLI. Should be running on a VPS/remote server.")]
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
        Command::Up => commands::up::run(),
        Command::Down => commands::down::run(),
    }
}
