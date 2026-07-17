use dialoguer::{Confirm, Input, Select, theme::ColorfulTheme};
use railyard_manifest::RailyardManifest;
use std::error::Error;
use std::path::Path;
use std::{env, fs};

use crate::config::{ServerConfig, list_servers, record_project_binding};
use crate::context::ExecContext;
use crate::http;
use crate::resolve::{
    MANIFEST_FILE, ProjectBinding, find_manifest, manifest_in, parse_manifest, project_binding,
    resolve_server,
};

/// Create a project on a server and link this directory to it
#[derive(clap::Args)]
pub(crate) struct Args {
    /// Project name; otherwise prompts when creating a manifest
    name: Option<String>,
    #[arg(long)]
    server: Option<String>,
}

pub(crate) fn run(args: Args, ctx: ExecContext) -> Result<(), Box<dyn Error>> {
    let Args {
        name,
        server: server_flag,
    } = args;
    let cwd = env::current_dir()?;
    let (manifest_path, mut manifest, manifest_raw) = match manifest_in(&cwd)? {
        Some((path, raw)) => {
            let manifest = parse_manifest(&path, &raw)?;
            (path, manifest, Some(raw))
        }
        None => {
            confirm_nested_init(&cwd, ctx)?;
            (cwd.join(MANIFEST_FILE), RailyardManifest::default(), None)
        }
    };
    let manifest_exists = manifest_raw.is_some();

    // Rerunning init in a linked directory is a no-op, not a chance to fork
    // the project; moving to another server goes through `railyard unlink`.
    if let Some(project) = &manifest.project
        && let Some(id) = &project.id
    {
        match project_binding(id)? {
            ProjectBinding::Bound(bound_name, _) => {
                if let Some(requested) = &server_flag
                    && *requested != bound_name
                {
                    return Err(format!(
                        "project {} ({id}) is already linked to {bound_name}; run `railyard \
                         unlink` first to link it to another server",
                        project.name
                    )
                    .into());
                }
                println!(
                    "Project {} ({id}) is already linked to {bound_name} — nothing to do.",
                    project.name
                );
                println!("To link this project to another server, run `railyard unlink` first.");
                return Ok(());
            }
            ProjectBinding::Stale(stale) => eprintln!(
                "note: ignoring the link to {stale}, which no longer exists on this machine"
            ),
            ProjectBinding::Unbound => {}
        }
    }

    let project_name = resolve_project_name(name, &manifest, manifest_exists, ctx)?;
    let (server_name, server) = resolve_server_for_init(server_flag, ctx)?;

    // A manifest can arrive with a project.id already in it — a cloned repo
    // someone else deployed, or a project being brought to a second server.
    // Reuse that id when it is available. If it already exists, let the user
    // explicitly adopt the server project or create a separate local project.
    let existing_id = manifest.project.as_ref().and_then(|p| p.id.clone());
    let mut id_to_create = existing_id.as_deref();
    let mut name_to_create = project_name.clone();
    if let Some(id) = &existing_id {
        let projects = http::list_projects(&server)?;
        if let Some(server_project) = projects.into_iter().find(|project| project.id == *id) {
            if !ctx.interactive {
                return Err(format!(
                    "project {} ({id}) already exists on {server_name}; rerun `railyard init` \
                     interactively to link this directory or create a new project",
                    server_project.name
                )
                .into());
            }

            let choices = [
                format!("Link this directory to project {}", server_project.name),
                "Create a new project and replace the ID in this directory".to_string(),
            ];
            let choice = Select::with_theme(&ColorfulTheme::default())
                .with_prompt(format!(
                    "Project {} ({id}) already exists on {server_name}. What would you like to do?",
                    server_project.name
                ))
                .items(&choices)
                .default(0)
                .interact()?;

            if choice == 0 {
                manifest.link_project(&server_project.name, &server_project.id);
                write_manifest(&manifest_path, &manifest, manifest_raw.as_deref())?;
                record_project_binding(&server_project.id, &server_name)?;
                println!(
                    "Linked this directory to project {} ({}) on {server_name}",
                    server_project.name, server_project.id
                );
                return Ok(());
            }

            id_to_create = None;
            if name_to_create == server_project.name {
                name_to_create = Input::with_theme(&ColorfulTheme::default())
                    .with_prompt("Name for the new project")
                    .default(format!("{project_name}-new"))
                    .interact_text()?;
            }
            println!(
                "Creating project {name_to_create} on {server_name} ({}) with a new id",
                server.server_url
            );
        } else {
            println!(
                "Creating project {project_name} on {server_name} ({}) with existing id {id}",
                server.server_url
            );
        }
    } else {
        println!(
            "Creating project {project_name} on {server_name} ({})",
            server.server_url
        );
    }
    let created = http::create_project(&server, &name_to_create, id_to_create)?;

    manifest.link_project(&created.name, &created.id);
    write_manifest(&manifest_path, &manifest, manifest_raw.as_deref())?;
    record_project_binding(&created.id, &server_name)?;

    println!(
        "Created project {} ({}) and linked {}",
        created.name,
        created.id,
        manifest_name(&manifest_path)
    );
    Ok(())
}

fn manifest_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Linking re-serializes the whole manifest as plain JSON, so comments in a
/// relaxed-syntax file can't survive; say so rather than dropping them
/// silently.
fn write_manifest(
    path: &Path,
    manifest: &RailyardManifest,
    raw: Option<&str>,
) -> Result<(), Box<dyn Error>> {
    fs::write(path, manifest.to_json_string())?;
    if let Some(raw) = raw
        && serde_json::from_str::<serde_json::Value>(raw).is_err()
    {
        eprintln!(
            "note: rewrote {} as plain JSON; comments were not preserved",
            manifest_name(path)
        );
    }
    Ok(())
}

/// Scaffolding a manifest inside an existing project's tree is almost
/// always `init` run from the wrong directory, so ask before creating a
/// nested project.
fn confirm_nested_init(cwd: &Path, ctx: ExecContext) -> Result<(), Box<dyn Error>> {
    let Some(parent) = cwd.parent() else {
        return Ok(());
    };
    let Some((found, raw)) = find_manifest(parent)? else {
        return Ok(());
    };
    // A broken ancestor manifest shouldn't block init here; name the file
    // and let the user decide.
    let project = parse_manifest(&found, &raw)
        .ok()
        .and_then(|manifest| manifest.project)
        .map(|project| format!(" (project {})", project.name))
        .unwrap_or_default();

    if !ctx.interactive {
        return Err(format!(
            "found {}{project} in a parent directory; init here would create a separate \
             nested project — run it from that directory, or rerun interactively to confirm",
            found.display()
        )
        .into());
    }
    let confirmed = Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt(format!(
            "Found {}{project} in a parent directory. Are you sure you want to create a \
             separate project here?",
            found.display()
        ))
        .default(false)
        .interact()?;
    if !confirmed {
        return Err("init cancelled".into());
    }
    Ok(())
}

/// Explicit arg wins unless the manifest already names a different project.
/// A new manifest prompts on a TTY, defaulting to the directory name squeezed
/// into the same DNS-label shape that `project.name` validation demands.
fn resolve_project_name(
    explicit: Option<String>,
    manifest: &RailyardManifest,
    manifest_exists: bool,
    ctx: ExecContext,
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
    if !manifest_exists && ctx.interactive {
        return Ok(Input::with_theme(&ColorfulTheme::default())
            .with_prompt("Project name")
            .default(name)
            .interact_text()?);
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

/// `init` is where a server gets chosen for a project, so it alone may
/// prompt: with several servers and no `--server`, show a picker on a TTY.
fn resolve_server_for_init(
    explicit: Option<String>,
    ctx: ExecContext,
) -> Result<(String, ServerConfig), Box<dyn Error>> {
    let mut servers = list_servers()?;
    if explicit.is_some() || servers.len() < 2 || !ctx.interactive {
        return resolve_server(explicit);
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
