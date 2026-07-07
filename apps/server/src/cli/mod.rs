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
    /// Start the server daemon.
    Up {
        /// Run in the foreground instead of daemonizing (for dev and process
        /// supervisors).
        #[arg(long)]
        foreground: bool,
    },
    /// Stop the server daemon.
    Down,
    /// Stop and start the server daemon.
    Restart,
    /// Show whether the server daemon is running.
    Status,
    /// Manage users (one user = one device keypair).
    User {
        #[command(subcommand)]
        command: UserCommand,
    },
}

#[derive(Subcommand)]
enum UserCommand {
    /// Create a user and print its single-use invite blob.
    Add { name: String },
    /// List users and whether their invite has been redeemed.
    List,
    /// Delete a user, revoking its key.
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
