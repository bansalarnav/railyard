mod auth;
mod commands;
mod config;
mod http;
mod resolve;

use clap::{Parser, Subcommand};
use std::error::Error;

#[derive(Parser)]
#[command(name = "railyard")]
#[command(about = "Railyard client CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Login {
        /// An invite blob, or an SSH target (user@host) to mint one on
        target: String,
        /// Local name for this server; defaults to the name embedded in the invite
        #[arg(long)]
        name: Option<String>,
        /// User to create when logging in over SSH; defaults to your local username
        #[arg(long)]
        user: Option<String>,
    },
    /// Show every identity this machine holds and which one commands here would use
    Whoami {
        /// Only check this server
        #[arg(long)]
        server: Option<String>,
    },
    /// Create a project on a server and link this directory to it
    Init {
        /// Project name; otherwise prompts when creating a manifest
        name: Option<String>,
        #[arg(long)]
        server: Option<String>,
    },
    /// Validate the manifest and pack the repository for deploy
    Up {
        #[arg(long)]
        server: Option<String>,
    },
    /// Pick one of your servers and link this directory's project to it
    Link,
    /// Forget which server this directory's project is linked to
    Unlink,
    User {
        #[command(subcommand)]
        command: UserCommand,
    },
}

#[derive(Subcommand)]
enum UserCommand {
    /// Invite a user to the current project and print the invite blob
    Add {
        name: String,
        /// Invite a server-wide admin instead of a project user
        #[arg(long)]
        admin: bool,
        /// Use this server instead of resolving one
        #[arg(long)]
        server: Option<String>,
    },
    /// List a server's users (admin only)
    List {
        #[arg(long)]
        server: Option<String>,
    },
    /// Remove a user and revoke its keys (admin only)
    Remove {
        name: String,
        #[arg(long)]
        server: Option<String>,
    },
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    match Cli::parse().command {
        Commands::Login { target, name, user } => commands::login::run(&target, name, user),
        Commands::Whoami { server } => commands::whoami::run(server),
        Commands::Init { name, server } => commands::init::run(name, server),
        Commands::Up { server } => commands::up::run(server),
        Commands::Link => commands::link::run(),
        Commands::Unlink => commands::unlink::run(),
        Commands::User { command } => match command {
            UserCommand::Add {
                name,
                admin,
                server,
            } => commands::user::add(&name, admin, server),
            UserCommand::List { server } => commands::user::list(server),
            UserCommand::Remove { name, server } => commands::user::remove(&name, server),
        },
    }
}
