mod auth;
mod commands;
mod config;
mod context;
mod http;
mod resolve;

use clap::{Parser, Subcommand};
use std::error::Error;

use context::ExecContext;

#[derive(Parser)]
#[command(name = "railyard")]
#[command(about = "Railyard client CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Login(commands::login::Args),
    Whoami(commands::whoami::Args),
    Init(commands::init::Args),
    /// Pick one of your servers and link this directory's project to it
    Link,
    /// Forget which server this directory's project is linked to
    Unlink,
    User(commands::user::Args),
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let ctx = ExecContext::detect();
    match Cli::parse().command {
        Commands::Login(args) => commands::login::run(args),
        Commands::Whoami(args) => commands::whoami::run(args),
        Commands::Init(args) => commands::init::run(args, ctx),
        Commands::Link => commands::link::run(ctx),
        Commands::Unlink => commands::unlink::run(ctx),
        Commands::User(args) => commands::user::run(args, ctx),
    }
}
