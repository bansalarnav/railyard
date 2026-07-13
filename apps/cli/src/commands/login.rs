use railyard_auth::{INVITE_BLOB_PREFIX, InvitePayload, WhoamiResponse, unix_timestamp};
use std::env;
use std::error::Error;
use std::process::Command;

use crate::auth::{generate_signing_key, public_key_base64};
use crate::config::{
    ServerConfig, list_servers, read_server, rebind_projects, record_project_binding,
    remove_server, sanitize_server_name, write_server, write_signing_key,
};
use crate::http;

pub(crate) fn run(
    target: &str,
    server_name: Option<String>,
    user_flag: Option<String>,
) -> Result<(), Box<dyn Error>> {
    if target.starts_with(INVITE_BLOB_PREFIX) {
        login(target, server_name)
    } else {
        login_ssh(target, server_name, user_flag)
    }
}

/// `login user@host`: bootstrap sugar for admins with SSH access — run
/// `railyard-server user add` on the box and redeem the resulting blob
/// locally in one step.
fn login_ssh(
    target: &str,
    server_name: Option<String>,
    user_flag: Option<String>,
) -> Result<(), Box<dyn Error>> {
    let user_name = match user_flag {
        Some(name) => name,
        None => {
            let local = env::var("USER")
                .or_else(|_| env::var("USERNAME"))
                .unwrap_or_default();
            let name = sanitize_user_name(&local);
            if name.is_empty() {
                return Err("could not derive a user name from $USER; pass --user <name>".into());
            }
            name
        }
    };

    println!("Creating user {user_name} on {target} over SSH…");
    let output = Command::new("ssh")
        .arg(target)
        .args(["railyard-server", "user", "add", &user_name])
        .output()?;

    if !output.status.success() {
        return Err(format!(
            "`ssh {target} railyard-server user add {user_name}` failed:\n{}",
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let blob = stdout
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with(INVITE_BLOB_PREFIX))
        .ok_or("the remote `user add` printed no invite blob")?;

    login(blob, server_name)
}

/// Squeeze a local username into the server's user-name charset.
fn sanitize_user_name(raw: &str) -> String {
    raw.to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_'))
        .collect()
}

fn login(blob: &str, server_name: Option<String>) -> Result<(), Box<dyn Error>> {
    let invite = InvitePayload::parse(blob)?;
    if invite.expires_at <= unix_timestamp() {
        return Err("this invite has expired; ask for a new one".into());
    }

    // A project invite for a server where an existing identity already
    // covers that project (admin, or scoped to it, with a key that still
    // works) adds nothing — record the binding and leave the invite alone.
    // A dead key falls through to redemption, which is how a lost device
    // gets replaced.
    if let Some(project) = &invite.project
        && let Some((entry_name, identity)) = existing_access(&invite.server_url, &project.id)?
    {
        record_project_binding(&project.id, &entry_name)?;
        println!(
            "Already have access to project {} on {} as user {} (server {entry_name}); \
             linked the project — invite left unredeemed",
            project.name, invite.server_url, identity.name
        );
        return Ok(());
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

    // An admin identity covers every project on its server, so redeeming an
    // admin invite makes any project-scoped entry for the same server
    // redundant (the one way to hold both: project user first, promoted to
    // admin later). Drop those entries and move their bindings here.
    if invite.project.is_none() {
        supersede_project_entries(&invite.server_url, &server_name)?;
    }

    // A project-scoped invite says exactly which server owns the project, so
    // record the binding now — a cloned repo naming that project id then
    // resolves immediately, no `init`/`link` step.
    if let Some(project) = &invite.project {
        record_project_binding(&project.id, &server_name)?;
        println!(
            "Linked project {} ({}) to {server_name}",
            project.name, project.id
        );
    }

    Ok(())
}

/// Remove project-scoped entries for `server_url` that the new admin entry
/// makes redundant, repointing their project bindings at it. Entries whose
/// scope can't be confirmed (unreachable, rejected key) are left alone.
fn supersede_project_entries(server_url: &str, admin_entry: &str) -> Result<(), Box<dyn Error>> {
    for (name, server) in list_servers()? {
        if name == admin_entry || server.server_url != server_url {
            continue;
        }
        let project_scoped = matches!(
            http::whoami(&server),
            Ok(http::WhoamiOutcome::Identity(identity)) if identity.project_id.is_some()
        );
        if !project_scoped {
            continue;
        }

        let relinked = rebind_projects(&name, admin_entry)?;
        remove_server(&name)?;
        match relinked {
            0 => println!("Removed project entry {name}; {admin_entry} covers it now"),
            n => println!(
                "Removed project entry {name} and relinked {n} project(s) to {admin_entry}"
            ),
        }
    }
    Ok(())
}

/// An existing identity on `server_url` whose live-checked scope covers
/// `project_id`: an admin, or a user scoped to that same project.
fn existing_access(
    server_url: &str,
    project_id: &str,
) -> Result<Option<(String, WhoamiResponse)>, Box<dyn Error>> {
    for (name, server) in list_servers()? {
        if server.server_url != server_url {
            continue;
        }
        if let Ok(http::WhoamiOutcome::Identity(identity)) = http::whoami(&server)
            && (identity.project_id.is_none() || identity.project_id.as_deref() == Some(project_id))
        {
            return Ok(Some((name, identity)));
        }
    }
    Ok(None)
}

/// Explicit --name wins; otherwise derive from the invite: the project name
/// for project-scoped invites (so a project identity does not collide with
/// an admin entry for the same server), then the server's human name, then
/// the host of `server_url`.
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

    if let Some(project) = &invite.project {
        let name = sanitize_server_name(&project.name);
        if !name.is_empty() {
            return Ok(name);
        }
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
