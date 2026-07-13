use dialoguer::{Confirm, Select, theme::ColorfulTheme};
use railyard_auth::unix_timestamp;
use std::error::Error;
use std::io;
use std::io::IsTerminal;

use crate::config::{ServerConfig, list_servers, read_server};
use crate::http;
use crate::resolve::{
    MANIFEST_FILE, confirmed_linked_project, resolve_project_server, resolve_server,
};

/// Invite to the project this directory is linked to; `--server` only pins
/// which server entry to use, like every other project command. `--admin`
/// switches to a server-wide (admin) invite. Either way the server only
/// honors the request from an admin key.
pub(crate) fn add(
    name: &str,
    admin: bool,
    server_flag: Option<String>,
) -> Result<(), Box<dyn Error>> {
    if admin {
        return add_admin(name, server_flag);
    }

    let Some(project) = confirmed_linked_project()? else {
        // No project to scope the invite to. On a TTY, offer the only other
        // invite this command can mint — but never silently escalate.
        if server_flag.is_none()
            && io::stdin().is_terminal()
            && Confirm::with_theme(&ColorfulTheme::default())
                .with_prompt(format!(
                    "No project is linked in this directory. Create a server-wide admin \
                     invite for {name} instead?"
                ))
                .default(false)
                .interact()?
        {
            return add_admin(name, None);
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
        None => resolve_project_server(&project)?,
    };
    let created = http::create_user(&server, name, Some(&project.id))?;
    println!(
        "Created user {name} scoped to project {} on {server_name}.",
        project.name
    );
    print_invite(&created.invite_blob);
    Ok(())
}

fn add_admin(name: &str, server_flag: Option<String>) -> Result<(), Box<dyn Error>> {
    let (server_name, server) = resolve_admin_server(server_flag)?;
    let created = http::create_user(&server, name, None)?;
    println!("Created admin user {name} with access to all of {server_name}.");
    print_invite(&created.invite_blob);
    Ok(())
}

fn print_invite(blob: &str) {
    println!("Single-use invite, expires in 24h. Redeem with `railyard login <blob>`:");
    println!();
    println!("{blob}");
}

pub(crate) fn list(server_flag: Option<String>) -> Result<(), Box<dyn Error>> {
    let (server_name, server) = resolve_server(server_flag)?;
    let users = http::list_users(&server)?;
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

pub(crate) fn remove(name: &str, server_flag: Option<String>) -> Result<(), Box<dyn Error>> {
    let (server_name, server) = resolve_server(server_flag)?;
    if http::remove_user(&server, name)? {
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
fn resolve_admin_server(
    explicit: Option<String>,
) -> Result<(String, ServerConfig), Box<dyn Error>> {
    let servers = list_servers()?;
    if explicit.is_some() || servers.len() < 2 || !io::stdin().is_terminal() {
        return resolve_server(explicit);
    }

    let mut candidates: Vec<(String, ServerConfig)> = servers
        .into_iter()
        .filter(|(_, server)| is_admin_identity(server))
        .collect();
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
fn is_admin_identity(server: &ServerConfig) -> bool {
    matches!(
        http::whoami(server),
        Ok(http::WhoamiOutcome::Identity(identity)) if identity.project_id.is_none()
    )
}
