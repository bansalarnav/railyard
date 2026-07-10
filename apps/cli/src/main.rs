mod auth;
mod config;
mod http;

use clap::{Parser, Subcommand};
use railyard_auth::{InvitePayload, unix_timestamp};
use railyard_manifest::RailyardManifest;
use std::error::Error;
use std::io::{IsTerminal, Write};
use std::path::Path;
use std::{env, fs, io};

use auth::{generate_signing_key, public_key_base64};
use config::{
    ClientProfile, list_profiles, read_profile, record_project_binding, sanitize_profile_name,
    write_profile, write_signing_key,
};

const MANIFEST_FILE: &str = ".railyard.json";

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
        blob: String,
        #[arg(long)]
        profile: Option<String>,
    },
    /// Create a project on a server and link this directory to it
    Init {
        /// Project name; defaults to the manifest's project name, then the directory name
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
        #[arg(long)]
        profile: Option<String>,
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
        Commands::Init { name, profile } => init(name, profile),
        Commands::Services { command } => match command {
            ServicesCommand::List { profile } => {
                let (_, profile) = resolve_profile(profile)?;
                let services = http::list_services(&profile)?;
                println!("{}", serde_json::to_string_pretty(&services)?);
                Ok(())
            }
        },
    }
}

fn init(name: Option<String>, profile_flag: Option<String>) -> Result<(), Box<dyn Error>> {
    let manifest_path = Path::new(MANIFEST_FILE);
    let mut manifest = match fs::read_to_string(manifest_path) {
        Ok(raw) => railyard_manifest::parse(&raw)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => RailyardManifest::default(),
        Err(error) => return Err(error.into()),
    };

    if let Some(id) = manifest.project.as_ref().and_then(|p| p.id.as_deref()) {
        return Err(format!(
            "{MANIFEST_FILE} is already linked to project {id}; remove project.id to re-init"
        )
        .into());
    }

    let project_name = resolve_project_name(name, &manifest)?;
    let (profile_name, profile) = resolve_profile(profile_flag)?;

    println!(
        "Creating project {project_name} on {profile_name} ({})",
        profile.server_url
    );
    let created = http::create_project(&profile, &project_name)?;

    manifest.link_project(&created.name, &created.id);
    fs::write(manifest_path, manifest.to_json_string())?;
    record_project_binding(&created.id, &profile_name)?;

    println!(
        "Created project {} ({}) and linked {MANIFEST_FILE}",
        created.name, created.id
    );
    Ok(())
}

/// Explicit arg wins unless the manifest already names a different project;
/// then the manifest's name; then the directory name squeezed into a DNS
/// label (the same shape `project.name` validation demands).
fn resolve_project_name(
    explicit: Option<String>,
    manifest: &RailyardManifest,
) -> Result<String, Box<dyn Error>> {
    let manifest_name = manifest.project.as_ref().map(|p| p.name.clone());

    if let Some(raw) = explicit {
        if let Some(existing) = manifest_name
            && existing != raw
        {
            return Err(format!(
                "{MANIFEST_FILE} already names this project {existing}; rerun without a name or edit the file"
            )
            .into());
        }
        return Ok(raw);
    }

    if let Some(existing) = manifest_name {
        return Ok(existing);
    }

    let dir = env::current_dir()?;
    let dir_name = dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let name = sanitize_project_name(dir_name);
    if name.is_empty() {
        return Err(
            "could not derive a project name from this directory; run `railyard init <name>`"
                .into(),
        );
    }
    Ok(name)
}

fn sanitize_project_name(raw: &str) -> String {
    let mut name = String::new();
    for char in raw.to_lowercase().chars() {
        if char.is_ascii_lowercase() || char.is_ascii_digit() {
            name.push(char);
        } else if !name.is_empty() && !name.ends_with('-') {
            name.push('-');
        }
    }
    name.truncate(63);
    name.trim_matches('-').to_string()
}

/// `--profile` flag, then `RAILYARD_PROFILE`, then the sole existing profile.
/// With several profiles and no selection, prompt on a TTY, else error —
/// this is where a server gets chosen when there is no project binding yet.
fn resolve_profile(explicit: Option<String>) -> Result<(String, ClientProfile), Box<dyn Error>> {
    if let Some(name) = explicit.or_else(|| env::var("RAILYARD_PROFILE").ok()) {
        let profile = read_profile(&name)
            .map_err(|error| format!("could not read profile {name}: {error}"))?;
        return Ok((name, profile));
    }

    let mut profiles = list_profiles()?;
    match profiles.len() {
        0 => Err("no profiles found; run `railyard login <blob>` first".into()),
        1 => Ok(profiles.remove(0)),
        _ => pick_profile(profiles),
    }
}

fn pick_profile(
    mut profiles: Vec<(String, ClientProfile)>,
) -> Result<(String, ClientProfile), Box<dyn Error>> {
    if !io::stdin().is_terminal() {
        let names: Vec<String> = profiles.iter().map(|(name, _)| name.clone()).collect();
        return Err(format!(
            "multiple profiles exist ({}); pass --profile <name>",
            names.join(", ")
        )
        .into());
    }

    eprintln!("Multiple profiles:");
    for (index, (name, profile)) in profiles.iter().enumerate() {
        eprintln!("  {}. {name} ({})", index + 1, profile.server_url);
    }
    eprint!("Choose a profile [1-{}]: ", profiles.len());
    io::stderr().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let choice: usize = input
        .trim()
        .parse()
        .map_err(|_| "not a number; pass --profile <name> to skip the prompt")?;
    if choice == 0 || choice > profiles.len() {
        return Err(format!("pick a number between 1 and {}", profiles.len()).into());
    }

    Ok(profiles.remove(choice - 1))
}

fn login(blob: &str, profile_name: Option<String>) -> Result<(), Box<dyn Error>> {
    let invite = InvitePayload::parse(blob)?;
    if invite.expires_at <= unix_timestamp() {
        return Err("this invite has expired; ask for a new one".into());
    }

    let profile_name = resolve_profile_name(profile_name, &invite)?;

    // Re-redeeming against the same server just rotates this machine's key;
    // a different server squatting on the name needs an explicit --profile.
    if let Ok(existing) = read_profile(&profile_name)
        && existing.server_url != invite.server_url
    {
        return Err(format!(
            "profile {profile_name} already points at {}; pass --profile <name> to pick another name",
            existing.server_url
        )
        .into());
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

/// Explicit --profile wins; otherwise derive from the invite: the server's
/// human name, falling back to the host of `server_url`.
fn resolve_profile_name(
    explicit: Option<String>,
    invite: &InvitePayload,
) -> Result<String, Box<dyn Error>> {
    if let Some(raw) = explicit {
        let name = sanitize_profile_name(&raw);
        if name.is_empty() {
            return Err("profile name has no usable characters".into());
        }
        return Ok(name);
    }

    let name = sanitize_profile_name(&invite.server_name);
    if !name.is_empty() {
        return Ok(name);
    }

    if let Ok(url) = url::Url::parse(&invite.server_url)
        && let Some(host) = url.host_str()
    {
        let name = sanitize_profile_name(host);
        if !name.is_empty() {
            return Ok(name);
        }
    }

    Err("could not derive a profile name from this invite; pass --profile <name>".into())
}
