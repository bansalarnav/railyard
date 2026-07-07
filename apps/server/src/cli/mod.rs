mod daemon;
mod user;

use clap::{Parser, Subcommand};
use std::io;

#[derive(Parser)]
#[command(name = "railyard-server")]
#[command(about = "Railyard server daemon control CLI. Should be running on a VPS/remote server.")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Up {
        #[arg(long)]
        foreground: bool,
    },
    Down,
    Restart,
    Status,
    User {
        #[command(subcommand)]
        command: UserCommand,
    },
}

#[derive(Subcommand)]
enum UserCommand {
    Add { name: String },
    List,
    Remove { name: String },
}

pub(crate) fn run() -> io::Result<()> {
    match Cli::parse().command {
        Command::Up { foreground } => daemon::up(foreground),
        Command::Down => daemon::down(),
        Command::Restart => daemon::restart(),
        Command::Status => {
            daemon::status();
            Ok(())
        }
        Command::User { command } => match command {
            UserCommand::Add { name } => user::add(&name),
            UserCommand::List => user::list(),
            UserCommand::Remove { name } => user::remove(&name),
        },
    }
}
