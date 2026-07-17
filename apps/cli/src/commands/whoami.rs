use railyard_auth::WhoamiResponse;
use std::error::Error;

use crate::config::list_servers;
use crate::http;
use crate::resolve::{ProjectBinding, linked_project, project_binding};

/// Show every identity this machine holds and which one commands here would use
#[derive(clap::Args)]
pub(crate) struct Args {
    /// Only check this server
    #[arg(long)]
    server: Option<String>,
}

/// One row per server entry, queried live so the answer reflects what the
/// server believes (a revoked key shows up here, not in local config). The
/// starred row is what commands in the current directory would use, computed
/// with the same resolution rules those commands apply.
pub(crate) async fn run(args: Args) -> Result<(), Box<dyn Error>> {
    let server_flag = args.server;
    let mut servers = list_servers()?;
    if servers.is_empty() {
        return Err("no servers found; run `railyard login <blob>` first".into());
    }

    let (selected, note) = if let Some(name) = &server_flag {
        servers.retain(|(entry, _)| entry == name);
        if servers.is_empty() {
            return Err(format!("no server named {name}").into());
        }
        (Some(name.clone()), format!("selected by --server {name}"))
    } else if let Some(project) = linked_project()? {
        // Report-only: whoami never prompts, so it checks the binding rather
        // than resolving (which may offer to link).
        let via = match &project.manifest_path {
            Some(path) => format!(", manifest at {}", path.display()),
            None => String::new(),
        };
        match project_binding(&project.id) {
            Ok(ProjectBinding::Bound(name, _)) => (
                Some(name),
                format!(
                    "selected for this directory (project {}, {}{via})",
                    project.name, project.id
                ),
            ),
            Ok(ProjectBinding::Stale(stale)) => (
                None,
                format!(
                    "project {} ({}{via}) is linked to {stale}, which no longer exists here; \
                     any project command re-offers the link, `railyard unlink` forgets it",
                    project.name, project.id
                ),
            ),
            Ok(ProjectBinding::Unbound) => (
                None,
                format!(
                    "project {} ({}{via}) is not linked here yet",
                    project.name, project.id
                ),
            ),
            Err(error) => (None, error.to_string()),
        }
    } else if servers.len() == 1 {
        (
            Some(servers[0].0.clone()),
            "selected as the only known server".to_string(),
        )
    } else {
        (
            None,
            "no project in this directory; commands here need --server <name>".to_string(),
        )
    };

    for (name, server) in &servers {
        let marker = if selected.as_deref() == Some(name.as_str()) {
            "*"
        } else {
            " "
        };
        let who = match http::whoami(server).await {
            Ok(http::WhoamiOutcome::Identity(identity)) => describe_identity(&identity),
            Ok(http::WhoamiOutcome::Rejected(reason)) => format!("key rejected {reason}"),
            Ok(http::WhoamiOutcome::Unreachable) => "unreachable".to_string(),
            Err(error) => format!("error: {error}"),
        };
        println!("{marker} {name}\t{}\t{who}", server.server_url);
    }

    println!();
    println!("{note}");
    Ok(())
}

fn describe_identity(identity: &WhoamiResponse) -> String {
    let scope = match (&identity.project_id, &identity.project_name) {
        (None, _) => "admin".to_string(),
        (Some(id), None) => format!("project {id}"),
        (Some(_), Some(project)) => format!("project {project}"),
    };
    format!("user {} — {scope}", identity.name)
}
