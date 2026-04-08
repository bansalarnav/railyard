mod auth;
mod config;
mod http;

use clap::{Parser, Subcommand};
use std::error::Error;
use std::process::Command;

use auth::{BootstrapResponse, generate_signing_key, public_key_base64};
use config::{
    ClientProfile, default_device_name, sanitize_profile_name, write_profile, write_signing_key,
};

#[derive(Parser)]
#[command(name = "aethon")]
#[command(about = "Aethon client CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Login {
        ssh_target: String,
        #[arg(long)]
        name: Option<String>,
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
        Commands::Login {
            ssh_target,
            name,
            profile,
        } => login(ssh_target, name, profile),
        Commands::Services { command } => match command {
            ServicesCommand::List { profile } => {
                let services = http::list_services(&profile)?;
                println!("{}", serde_json::to_string_pretty(&services)?);
                Ok(())
            }
        },
    }
}

fn login(
    ssh_target: String,
    device_name: Option<String>,
    profile_name: Option<String>,
) -> Result<(), Box<dyn Error>> {
    let device_name = device_name.unwrap_or_else(default_device_name);
    let device_name = device_name.replace(' ', "-");
    let profile_name = profile_name
        .unwrap_or_else(|| "default".to_string());
    let profile_name = sanitize_profile_name(&profile_name);
    let signing_key = generate_signing_key();
    let public_key = public_key_base64(&signing_key);
    let remote_command = format!(
        "aethon-server auth register-key --name {} --public-key {}",
        shell_escape(&device_name),
        shell_escape(&public_key)
    );

    let output = Command::new("ssh")
        .arg(&ssh_target)
        .arg(remote_command)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("ssh bootstrap failed: {stderr}").into());
    }

    let bootstrap: BootstrapResponse = serde_json::from_slice(&output.stdout)?;
    let key_path = write_signing_key(&bootstrap.key_id, &signing_key)?;

    write_profile(
        &profile_name,
        &ClientProfile {
            server_url: bootstrap.server_url.clone(),
            ssh_target,
            key_id: bootstrap.key_id.clone(),
            device_name: bootstrap.device_name.clone(),
            private_key_path: key_path.display().to_string(),
        },
    )?;

    println!(
        "Saved profile {} for {} ({})",
        profile_name, bootstrap.device_name, bootstrap.server_url
    );

    Ok(())
}

fn shell_escape(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}
