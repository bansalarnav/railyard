use dialoguer::{Confirm, Select, theme::ColorfulTheme};
use railyard_auth::unix_timestamp;
use std::error::Error;

use crate::config::{ServerConfig, list_servers, read_server};
use crate::context::ExecContext;
use crate::http;
use crate::resolve::{
    MANIFEST_FILE, confirmed_linked_project, resolve_project_server, resolve_server,
};

#[derive(clap::Args)]
pub(crate) struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(clap::Subcommand)]
enum Command {
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

pub(crate) async fn run(args: Args, ctx: ExecContext) -> Result<(), Box<dyn Error>> {
    match args.command {
        Command::Add {
            name,
            admin,
            server,
        } => add(&name, admin, server, ctx).await,
        Command::List { server } => list(server).await,
        Command::Remove { name, server } => remove(&name, server).await,
    }
}

/// Invite to the project this directory is linked to; `--server` only pins
/// which server entry to use, like every other project command. `--admin`
/// switches to a server-wide (admin) invite. Either way the server only
/// honors the request from an admin key.
async fn add(
    name: &str,
    admin: bool,
    server_flag: Option<String>,
    ctx: ExecContext,
) -> Result<(), Box<dyn Error>> {
    if admin {
        return add_admin(name, server_flag, ctx).await;
    }

    let Some(project) = confirmed_linked_project(ctx)? else {
        // No project to scope the invite to. On a TTY, offer the only other
        // invite this command can mint — but never silently escalate.
        if server_flag.is_none()
            && ctx.interactive
            && Confirm::with_theme(&ColorfulTheme::default())
                .with_prompt(format!(
                    "No project is linked in this directory. Create a server-wide admin \
                     invite for {name} instead?"
                ))
                .default(false)
                .interact()?
        {
            return add_admin(name, None, ctx).await;
        }
        return Err(format!(
            "no project linked in this directory ({MANIFEST_FILE} with a project.id); run \
             `railyard init` first, or pass --admin to invite someone to a whole server"
        )
        .into());
    };

    let (server_name, server) = match server_flag {
        Some(server_name) => {
            let server = read_server(&server_name)
                .map_err(|error| format!("could not read server {server_name}: {error}"))?;
            (server_name, server)
        }
        None => resolve_project_server(&project, ctx).await?,
    };
    let created = http::create_user(&server, name, Some(&project.id)).await?;
    println!(
        "Created user {name} scoped to project {} on {server_name}.",
        project.name
    );
    print_invite(&created.invite_blob);
    Ok(())
}

async fn add_admin(
    name: &str,
    server_flag: Option<String>,
    ctx: ExecContext,
) -> Result<(), Box<dyn Error>> {
    let (server_name, server) = resolve_admin_server(server_flag, ctx).await?;
    let created = http::create_user(&server, name, None).await?;
    println!("Created admin user {name} with access to all of {server_name}.");
    print_invite(&created.invite_blob);
    Ok(())
}

fn print_invite(blob: &str) {
    println!("Single-use invite, expires in 24h. Redeem with `railyard login <blob>`:");
    println!();
    println!("{blob}");
}

async fn list(server_flag: Option<String>) -> Result<(), Box<dyn Error>> {
    let (server_name, server) = resolve_server(server_flag)?;
    let users = http::list_users(&server).await?;
    if users.is_empty() {
        println!("No users on {server_name}.");
        return Ok(());
    }

    let now = unix_timestamp();
    for user in users {
        let status = if user.has_key { "active" } else { "invited" };
        let scope = user.project_id.as_deref().unwrap_or("admin");
        println!(
            "{}\t{}\t{}\t{}\tcreated {} ago",
            user.name,
            user.id,
            scope,
            status,
            format_age(now.saturating_sub(user.created_at))
        );
    }
    Ok(())
}

async fn remove(name: &str, server_flag: Option<String>) -> Result<(), Box<dyn Error>> {
    let (server_name, server) = resolve_server(server_flag)?;
    if http::remove_user(&server, name).await? {
        println!("Removed user {name} from {server_name} and revoked its keys.");
    } else {
        println!("No user named {name} on {server_name}.");
    }
    Ok(())
}

fn format_age(seconds: u64) -> String {
    match seconds {
        0..60 => format!("{seconds}s"),
        60..3600 => format!("{}m", seconds / 60),
        3600..86400 => format!("{}h", seconds / 3600),
        _ => format!("{}d", seconds / 86400),
    }
}

/// The server an admin invite lands on. `--server` wins, then the sole known
/// server. With several, a TTY narrows to the entries whose identity is an
/// admin (only admins can mint invites) and asks; non-interactive runs must
/// pass --server.
async fn resolve_admin_server(
    explicit: Option<String>,
    ctx: ExecContext,
) -> Result<(String, ServerConfig), Box<dyn Error>> {
    let servers = list_servers()?;
    if explicit.is_some() || servers.len() < 2 || !ctx.interactive {
        return resolve_server(explicit);
    }

    let mut candidates: Vec<(String, ServerConfig)> = Vec::new();
    for (name, server) in servers {
        if is_admin_identity(&server).await {
            candidates.push((name, server));
        }
    }
    match candidates.len() {
        0 => Err(
            "none of your servers answered with an admin identity, and only admins can mint \
             invites; check `railyard whoami`"
                .into(),
        ),
        1 => {
            let (name, server) = candidates.remove(0);
            let confirmed = Confirm::with_theme(&ColorfulTheme::default())
                .with_prompt(format!(
                    "Only {name} ({}) holds an admin identity here. Create the admin invite \
                     there?",
                    server.server_url
                ))
                .default(true)
                .interact()?;
            if !confirmed {
                return Err("no server chosen; pass --server <name> to pick one".into());
            }
            Ok((name, server))
        }
        _ => {
            let items: Vec<String> = candidates
                .iter()
                .map(|(name, server)| format!("{name} ({})", server.server_url))
                .collect();
            let choice = Select::with_theme(&ColorfulTheme::default())
                .with_prompt("Create the admin invite on which server?")
                .items(&items)
                .default(0)
                .interact()?;
            Ok(candidates.remove(choice))
        }
    }
}

/// Does this entry's key currently prove an admin on its server?
async fn is_admin_identity(server: &ServerConfig) -> bool {
    matches!(
        http::whoami(server).await,
        Ok(http::WhoamiOutcome::Identity(identity)) if identity.project_id.is_none()
    )
}
