mod auth;
mod config;
mod http;

use clap::{Parser, Subcommand};
use dialoguer::{Select, theme::ColorfulTheme};
use railyard_auth::{InvitePayload, unix_timestamp};
use railyard_manifest::RailyardManifest;
use std::error::Error;
use std::io::IsTerminal;
use std::path::Path;
use std::{env, fs, io};

use auth::{generate_signing_key, public_key_base64};
use config::{
    ServerConfig, list_servers, read_server, record_project_binding, sanitize_server_name,
    write_server, write_signing_key,
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
        /// Local name for this server; defaults to the name embedded in the invite
        #[arg(long)]
        name: Option<String>,
    },
    /// Create a project on a server and link this directory to it
    Init {
        /// Project name; defaults to the manifest's project name, then the directory name
        name: Option<String>,
        #[arg(long)]
        server: Option<String>,
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
    let cli = Cli::parse();

    match cli.command {
        Commands::Login { blob, name } => login(&blob, name),
        Commands::Init { name, server } => init(name, server),
        Commands::Services { command } => match command {
            ServicesCommand::List { server } => {
                let (_, server) = resolve_server(server)?;
                let services = http::list_services(&server)?;
                println!("{}", serde_json::to_string_pretty(&services)?);
                Ok(())
            }
        },
    }
}

fn init(name: Option<String>, server_flag: Option<String>) -> Result<(), Box<dyn Error>> {
    let manifest_path = Path::new(MANIFEST_FILE);
    let mut manifest = match fs::read_to_string(manifest_path) {
        Ok(raw) => railyard_manifest::parse(&raw)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => RailyardManifest::default(),
        Err(error) => return Err(error.into()),
    };

    let project_name = resolve_project_name(name, &manifest)?;
    let (server_name, server) = resolve_server(server_flag)?;

    // A manifest can arrive with a project.id already in it — most commonly a
    // cloned repo that someone else deployed. If the chosen server knows that
    // project this is a no-op link; if not, mint a fresh project here and
    // take over the id, leaving the original deployment untouched.
    if let Some(id) = manifest.project.as_ref().and_then(|p| p.id.clone()) {
        let projects = http::list_projects(&server)?;
        if let Some(existing) = projects.into_iter().find(|project| project.id == id) {
            record_project_binding(&id, &server_name)?;
            println!(
                "Project {} ({id}) already exists on {server_name}; linked this directory to it",
                existing.name
            );
            return Ok(());
        }
        println!(
            "{MANIFEST_FILE} points at project {id}, which {server_name} does not know — \
             creating a fresh project there instead"
        );
    }

    println!(
        "Creating project {project_name} on {server_name} ({})",
        server.server_url
    );
    let created = http::create_project(&server, &project_name)?;

    manifest.link_project(&created.name, &created.id);
    fs::write(manifest_path, manifest.to_json_string())?;
    record_project_binding(&created.id, &server_name)?;

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

/// `--server` flag, then `RAILYARD_SERVER`, then the sole known server.
/// With several servers and no selection, prompt on a TTY, else error —
/// this is where a server gets chosen when there is no project binding yet.
fn resolve_server(explicit: Option<String>) -> Result<(String, ServerConfig), Box<dyn Error>> {
    if let Some(name) = explicit.or_else(|| env::var("RAILYARD_SERVER").ok()) {
        let server =
            read_server(&name).map_err(|error| format!("could not read server {name}: {error}"))?;
        return Ok((name, server));
    }

    let mut servers = list_servers()?;
    match servers.len() {
        0 => Err("no servers found; run `railyard login <blob>` first".into()),
        1 => Ok(servers.remove(0)),
        _ => pick_server(servers),
    }
}

fn pick_server(
    mut servers: Vec<(String, ServerConfig)>,
) -> Result<(String, ServerConfig), Box<dyn Error>> {
    if !io::stdin().is_terminal() {
        let names: Vec<String> = servers.iter().map(|(name, _)| name.clone()).collect();
        return Err(format!(
            "multiple servers exist ({}); pass --server <name>",
            names.join(", ")
        )
        .into());
    }

    let items: Vec<String> = servers
        .iter()
        .map(|(name, server)| format!("{name} ({})", server.server_url))
        .collect();
    let choice = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Select a server")
        .items(&items)
        .default(0)
        .interact()?;

    Ok(servers.remove(choice))
}

fn login(blob: &str, server_name: Option<String>) -> Result<(), Box<dyn Error>> {
    let invite = InvitePayload::parse(blob)?;
    if invite.expires_at <= unix_timestamp() {
        return Err("this invite has expired; ask for a new one".into());
    }

    let server_name = resolve_server_name(server_name, &invite)?;

    // Re-redeeming against the same server just rotates this machine's key;
    // a different server squatting on the name needs an explicit --name.
    if let Ok(existing) = read_server(&server_name)
        && existing.server_url != invite.server_url
    {
        return Err(format!(
            "server {server_name} already points at {}; pass --name <name> to pick another name",
            existing.server_url
        )
        .into());
    }

    let signing_key = generate_signing_key();
    let redeemed = http::redeem_invite(&invite, &public_key_base64(&signing_key))?;
    let key_path = write_signing_key(&redeemed.key_id, &signing_key)?;

    write_server(
        &server_name,
        &ServerConfig {
            server_url: invite.server_url.clone(),
            key_id: redeemed.key_id.clone(),
            private_key_path: key_path.display().to_string(),
        },
    )?;

    println!(
        "Logged in to {} (key {}, server {})",
        invite.server_url, redeemed.key_id, server_name
    );

    Ok(())
}

/// Explicit --name wins; otherwise derive from the invite: the server's
/// human name, falling back to the host of `server_url`.
fn resolve_server_name(
    explicit: Option<String>,
    invite: &InvitePayload,
) -> Result<String, Box<dyn Error>> {
    if let Some(raw) = explicit {
        let name = sanitize_server_name(&raw);
        if name.is_empty() {
            return Err("server name has no usable characters".into());
        }
        return Ok(name);
    }

    let name = sanitize_server_name(&invite.server_name);
    if !name.is_empty() {
        return Ok(name);
    }

    if let Ok(url) = url::Url::parse(&invite.server_url)
        && let Some(host) = url.host_str()
    {
        let name = sanitize_server_name(host);
        if !name.is_empty() {
            return Ok(name);
        }
    }

    Err("could not derive a server name from this invite; pass --name <name>".into())
}
