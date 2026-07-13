use std::error::Error;

use crate::config::remove_project_binding;
use crate::resolve::{MANIFEST_FILE, confirmed_linked_project};

/// Drop the recorded project→server binding. The manifest keeps its
/// `project.id`, so `init` (or any project command) can link it again — to
/// the same server or another one.
pub(crate) fn run() -> Result<(), Box<dyn Error>> {
    let project = confirmed_linked_project()?.ok_or(format!(
        "no project linked in this directory ({MANIFEST_FILE} with a project.id)"
    ))?;

    match remove_project_binding(&project.id)? {
        Some(server_name) => println!(
            "Unlinked project {} ({}) from {server_name}; {MANIFEST_FILE} keeps its id, so \
             `railyard init` can link it again",
            project.name, project.id
        ),
        None => println!(
            "Project {} ({}) is not linked to any server — nothing to do.",
            project.name, project.id
        ),
    }
    Ok(())
}
