use dialoguer::{Select, theme::ColorfulTheme};
use std::error::Error;
use std::io;
use std::io::IsTerminal;

use crate::config::list_servers;
use crate::resolve::{
    MANIFEST_FILE, ProjectBinding, ProjectPresence, confirmed_linked_project, link_project,
    project_binding, server_project_presence,
};

/// Show every server this machine knows and link this directory's project to
/// the chosen one. Unlike the automatic offer in `offer_project_link`, the
/// list is not narrowed to servers that have the project — the choice is
/// checked after, so picking a server without it points at `init` instead of
/// silently recording a bad binding.
pub(crate) fn run() -> Result<(), Box<dyn Error>> {
    let project = confirmed_linked_project()?.ok_or(format!(
        "no project in this directory ({MANIFEST_FILE} with a project.id); run `railyard init` \
         to create one"
    ))?;

    match project_binding(&project.id)? {
        ProjectBinding::Bound(name, _) => {
            println!(
                "Project {} ({}) is already linked to {name} — nothing to do.",
                project.name, project.id
            );
            println!("To link this project to another server, run `railyard unlink` first.");
            return Ok(());
        }
        ProjectBinding::Stale(stale) => {
            eprintln!("note: ignoring the link to {stale}, which no longer exists on this machine")
        }
        ProjectBinding::Unbound => {}
    }

    let mut servers = list_servers()?;
    if servers.is_empty() {
        return Err("no servers found; run `railyard login <blob>` first".into());
    }
    if !io::stdin().is_terminal() {
        return Err(format!(
            "`railyard link` picks a server interactively; rerun on a TTY (project {}, {})",
            project.name, project.id
        )
        .into());
    }

    let items: Vec<String> = servers
        .iter()
        .map(|(name, server)| format!("{name} ({})", server.server_url))
        .collect();
    let choice = Select::with_theme(&ColorfulTheme::default())
        .with_prompt(format!("Link project {} to which server?", project.name))
        .items(&items)
        .default(0)
        .interact()?;
    let (name, server) = servers.remove(choice);

    match server_project_presence(&server, &project) {
        ProjectPresence::Present => {
            link_project(&project, name, server)?;
            Ok(())
        }
        ProjectPresence::Absent => Err(format!(
            "{name} does not have project {} ({}); run `railyard init --server {name}` to \
             create it there",
            project.name, project.id
        )
        .into()),
        ProjectPresence::Unknown(reason) => Err(format!(
            "could not check {name} for project {} ({reason}); restore access there and retry",
            project.name
        )
        .into()),
    }
}
