mod auth;
mod config;
mod http;

use clap::{Parser, Subcommand};
use railyard_auth::{InvitePayload, unix_timestamp};
use std::error::Error;

use auth::{generate_signing_key, public_key_base64};
use config::{ClientProfile, sanitize_profile_name, write_profile, write_signing_key};

#[derive(Parser)]
#[command(name = "railyard")]
#[command(about = "Railyard client CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Redeem an invite blob (`ryd-invite-v1.…`) and save a profile for the
    /// server it points at.
    Login {
        blob: String,
        #[arg(long)]
        profile: Option<String>,
    },
    Services {
        #[command(subcommand)]
        command: ServicesCommand,
    },
}

#[derive(Subcommand)]
enum ServicesCommand {
    List {
        #[arg(long, default_value = "default")]
        profile: String,
    },
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Login { blob, profile } => login(&blob, profile),
        Commands::Services { command } => match command {
            ServicesCommand::List { profile } => {
                let services = http::list_services(&profile)?;
                println!("{}", serde_json::to_string_pretty(&services)?);
                Ok(())
            }
        },
    }
}

fn login(blob: &str, profile_name: Option<String>) -> Result<(), Box<dyn Error>> {
    let invite = InvitePayload::parse(blob)?;
    if invite.expires_at <= unix_timestamp() {
        return Err("this invite has expired; ask for a new one".into());
    }

    let profile_name = sanitize_profile_name(&profile_name.unwrap_or_else(|| "default".into()));
    if profile_name.is_empty() {
        return Err("profile name has no usable characters".into());
    }

    let signing_key = generate_signing_key();
    let redeemed = http::redeem_invite(&invite, &public_key_base64(&signing_key))?;
    let key_path = write_signing_key(&redeemed.key_id, &signing_key)?;

    write_profile(
        &profile_name,
        &ClientProfile {
            server_url: invite.server_url.clone(),
            key_id: redeemed.key_id.clone(),
            private_key_path: key_path.display().to_string(),
        },
    )?;

    println!(
        "Logged in to {} (key {}, profile {})",
        invite.server_url, redeemed.key_id, profile_name
    );

    Ok(())
}
