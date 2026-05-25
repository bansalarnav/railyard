mod commands;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "railyard-server")]
#[command(about = "Railyard server daemon control CLI. Should be running on a VPS/remote server.")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Up,
    Down,
    Auth {
        #[command(subcommand)]
        command: AuthCommand,
    },
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
}

#[derive(Subcommand)]
enum AuthCommand {
    RegisterKey {
        #[arg(long)]
        name: String,
        #[arg(long)]
        public_key: String,
    },
    ListKeys,
    RevokeKey {
        key_id: String,
    },
}

#[derive(Subcommand)]
enum ConfigCommand {
    SetPublicUrl { public_url: String },
    Show,
}

pub fn run() {
    let cli = Cli::parse();

    match cli.command {
        Command::Up => commands::up::run(),
        Command::Down => commands::down::run(),
        Command::Auth { command } => match command {
            AuthCommand::RegisterKey { name, public_key } => {
                commands::auth::register_key::run(name, public_key)
            }
            AuthCommand::ListKeys => commands::auth::list_keys::run(),
            AuthCommand::RevokeKey { key_id } => commands::auth::revoke_key::run(key_id),
        },
        Command::Config { command } => match command {
            ConfigCommand::SetPublicUrl { public_url } => {
                commands::config::set_public_url::run(public_url)
            }
            ConfigCommand::Show => commands::config::show::run(),
        },
    }
}
