//! Which project and which server a command acts on: the manifest walk, the
//! recorded project→server bindings, and the shared `--server` resolution
//! rules every command applies.

use dialoguer::{Confirm, Select, theme::ColorfulTheme};
use railyard_manifest::{ManifestError, RailyardManifest};
use std::error::Error;
use std::path::{Path, PathBuf};
use std::{env, fs, io};

use crate::config::{
    ServerConfig, list_servers, read_project_binding, read_server, record_project_binding,
};
use crate::context::ExecContext;
use crate::http;

/// The spelling `init` writes; discovery also accepts the relaxed variants.
pub(crate) const MANIFEST_FILE: &str = ".railyard.json";
pub(crate) const MANIFEST_FILES: &[&str] =
    &[".railyard.json", ".railyard.jsonc", ".railyard.json5"];

pub(crate) struct LinkedProject {
    pub(crate) id: String,
    pub(crate) name: String,
    /// Set when the manifest was found in an ancestor directory, not here.
    pub(crate) manifest_path: Option<PathBuf>,
}

/// The project this directory belongs to, if any: the nearest manifest here
/// or in an ancestor, carrying a `project.id` (both written by
/// `railyard init`).
pub(crate) fn linked_project() -> Result<Option<LinkedProject>, Box<dyn Error>> {
    let cwd = env::current_dir()?;
    let Some((path, raw)) = find_manifest(&cwd)? else {
        return Ok(None);
    };

    let manifest = parse_manifest(&path, &raw)?;
    let manifest_path = (path.parent() != Some(cwd.as_path())).then_some(path);
    Ok(manifest.project.and_then(|project| {
        project.id.map(|id| LinkedProject {
            id,
            name: project.name,
            manifest_path,
        })
    }))
}

/// The manifest in `dir` itself, if any. Two spellings side by side would
/// make which one wins a guess, so that is an error, not a pick.
pub(crate) fn manifest_in(dir: &Path) -> Result<Option<(PathBuf, String)>, Box<dyn Error>> {
    let mut found: Vec<(PathBuf, String)> = Vec::new();
    for name in MANIFEST_FILES {
        match fs::read_to_string(dir.join(name)) {
            Ok(raw) => found.push((dir.join(name), raw)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    if found.len() > 1 {
        let names: Vec<&str> = found
            .iter()
            .filter_map(|(path, _)| path.file_name().and_then(|name| name.to_str()))
            .collect();
        return Err(format!(
            "multiple manifests in {}: {}; keep one and delete the rest",
            dir.display(),
            names.join(", ")
        )
        .into());
    }
    Ok(found.pop())
}

/// The nearest manifest at or above `start`. The walk stops at the first
/// file found — a manifest without a `project.id` still ends the search.
pub(crate) fn find_manifest(start: &Path) -> Result<Option<(PathBuf, String)>, Box<dyn Error>> {
    let mut dir = start.to_path_buf();
    loop {
        if let Some(found) = manifest_in(&dir)? {
            return Ok(Some(found));
        }
        if !dir.pop() {
            return Ok(None);
        }
    }
}

/// `.railyard.json` stays strict JSON; the `c`/`5` spellings get the
/// relaxed grammar.
pub(crate) fn parse_manifest(path: &Path, raw: &str) -> Result<RailyardManifest, ManifestError> {
    if path.extension().is_some_and(|ext| ext == "json") {
        railyard_manifest::parse(raw)
    } else {
        railyard_manifest::parse_relaxed(raw)
    }
}

/// `linked_project`, gated when the manifest lives in an ancestor directory:
/// confirm before acting on it, so a stray subdirectory never silently
/// targets the parent's project. Report-only callers (`whoami`) use
/// `linked_project` directly.
pub(crate) fn confirmed_linked_project(
    ctx: ExecContext,
) -> Result<Option<LinkedProject>, Box<dyn Error>> {
    let Some(project) = linked_project()? else {
        return Ok(None);
    };
    let Some(path) = &project.manifest_path else {
        return Ok(Some(project));
    };

    if !ctx.interactive {
        return Err(format!(
            "no manifest in this directory, but {} exists (project {}); rerun from its \
             directory to use it",
            path.display(),
            project.name
        )
        .into());
    }
    let confirmed = Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt(format!(
            "Use project {} from {}?",
            project.name,
            path.display()
        ))
        .default(true)
        .interact()?;
    Ok(confirmed.then_some(project))
}

/// The server for a linked project: the recorded binding, or — when none
/// exists (or the bound entry is gone) — an offer to link a server that
/// already has the project.
pub(crate) async fn resolve_project_server(
    project: &LinkedProject,
    ctx: ExecContext,
) -> Result<(String, ServerConfig), Box<dyn Error>> {
    match project_binding(&project.id)? {
        ProjectBinding::Bound(name, server) => Ok((name, server)),
        ProjectBinding::Stale(stale) => {
            eprintln!(
                "note: project {} was linked to {stale}, which no longer exists on this \
                 machine; looking for the project on your other servers",
                project.name
            );
            offer_project_link(project, ctx).await
        }
        ProjectBinding::Unbound => offer_project_link(project, ctx).await,
    }
}

pub(crate) enum ProjectBinding {
    Bound(String, ServerConfig),
    /// A binding exists but its server entry is gone (removed config); the
    /// name is kept for reporting.
    Stale(String),
    Unbound,
}

/// The binding recorded by `init` or a project-scoped `login`, checked
/// against the server entries that still exist.
pub(crate) fn project_binding(project_id: &str) -> Result<ProjectBinding, Box<dyn Error>> {
    let Some(name) = read_project_binding(project_id)? else {
        return Ok(ProjectBinding::Unbound);
    };
    match read_server(&name) {
        Ok(server) => Ok(ProjectBinding::Bound(name, server)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(ProjectBinding::Stale(name)),
        Err(error) => Err(format!("could not read server {name}: {error}").into()),
    }
}

/// No binding yet: quietly look for the project on every server this machine
/// could act on — admin identities, or one scoped to this very project — and
/// offer to link the match. `railyard link` is the explicit spelling of the
/// same step, for when the user wants to pick the server themselves.
async fn offer_project_link(
    project: &LinkedProject,
    ctx: ExecContext,
) -> Result<(String, ServerConfig), Box<dyn Error>> {
    let mut candidates: Vec<(String, ServerConfig)> = Vec::new();
    let mut unchecked: Vec<String> = Vec::new();
    for (name, server) in list_servers()? {
        match server_project_presence(&server, project).await {
            ProjectPresence::Present => candidates.push((name, server)),
            ProjectPresence::Absent => {}
            ProjectPresence::Unknown(reason) => unchecked.push(format!("{name} ({reason})")),
        }
    }

    match candidates.len() {
        // Recommending `init` while a server couldn't answer would risk
        // recreating a project that lives on the unreachable box.
        0 if !unchecked.is_empty() => Err(format!(
            "this project is not linked to a server on this machine, and no reachable server \
             has project {} ({}); could not check {} — restore access there before running \
             `railyard init`, which would create the project anew",
            project.name,
            project.id,
            unchecked.join(", ")
        )
        .into()),
        0 => Err(format!(
            "this project is not linked to a server on this machine, and none of your servers \
             have project {} ({}); run `railyard init` to create it",
            project.name, project.id
        )
        .into()),
        1 => {
            let (name, server) = candidates.remove(0);
            if !ctx.interactive {
                return Err(format!(
                    "found project {} ({}) on server {name}, but this directory is not linked \
                     to it; rerun interactively to link",
                    project.name, project.id
                )
                .into());
            }
            let confirmed = Confirm::with_theme(&ColorfulTheme::default())
                .with_prompt(format!(
                    "Found project {} on server {name}. Would you like to link it?",
                    project.name
                ))
                .default(true)
                .interact()?;
            if !confirmed {
                return Err("this project is not linked to a server on this machine".into());
            }
            link_project(project, name, server)
        }
        _ => {
            let names: Vec<String> = candidates.iter().map(|(name, _)| name.clone()).collect();
            if !ctx.interactive {
                return Err(format!(
                    "project {} ({}) exists on several servers ({}); rerun interactively to \
                     choose one to link",
                    project.name,
                    project.id,
                    names.join(", ")
                )
                .into());
            }
            let items: Vec<String> = candidates
                .iter()
                .map(|(name, server)| format!("{name} ({})", server.server_url))
                .collect();
            let choice = Select::with_theme(&ColorfulTheme::default())
                .with_prompt(format!(
                    "Project {} exists on several servers; link which one?",
                    project.name
                ))
                .items(&items)
                .default(0)
                .interact()?;
            let (name, server) = candidates.remove(choice);
            link_project(project, name, server)
        }
    }
}

pub(crate) fn link_project(
    project: &LinkedProject,
    name: String,
    server: ServerConfig,
) -> Result<(String, ServerConfig), Box<dyn Error>> {
    record_project_binding(&project.id, &name)?;
    println!("Linked project {} ({}) to {name}", project.name, project.id);
    Ok((name, server))
}

pub(crate) enum ProjectPresence {
    Present,
    Absent,
    /// Couldn't tell — the server was unreachable, the key rejected, or the
    /// listing failed. The reason is kept for reporting.
    Unknown(String),
}

/// Could this identity act on the project, and does its server have it?
pub(crate) async fn server_project_presence(
    server: &ServerConfig,
    project: &LinkedProject,
) -> ProjectPresence {
    match http::whoami(server).await {
        Ok(http::WhoamiOutcome::Identity(identity)) => match identity.project_id {
            Some(scoped) if scoped == project.id => ProjectPresence::Present,
            Some(_) => ProjectPresence::Absent,
            None => match http::list_projects(server).await {
                Ok(projects) if projects.iter().any(|p| p.id == project.id) => {
                    ProjectPresence::Present
                }
                Ok(_) => ProjectPresence::Absent,
                Err(error) => ProjectPresence::Unknown(format!("project listing failed: {error}")),
            },
        },
        Ok(http::WhoamiOutcome::Rejected(reason)) => {
            ProjectPresence::Unknown(format!("key rejected {reason}"))
        }
        Ok(http::WhoamiOutcome::Unreachable) => ProjectPresence::Unknown("unreachable".to_string()),
        Err(error) => ProjectPresence::Unknown(error.to_string()),
    }
}

/// `--server` flag, then the sole known server. Never prompts — commands
/// other than `init` must be told which server when several exist.
pub(crate) fn resolve_server(
    explicit: Option<String>,
) -> Result<(String, ServerConfig), Box<dyn Error>> {
    if let Some(name) = explicit {
        let server =
            read_server(&name).map_err(|error| format!("could not read server {name}: {error}"))?;
        return Ok((name, server));
    }

    let mut servers = list_servers()?;
    match servers.len() {
        0 => Err("no servers found; run `railyard login <blob>` first".into()),
        1 => Ok(servers.remove(0)),
        _ => {
            let names: Vec<String> = servers.iter().map(|(name, _)| name.clone()).collect();
            Err(format!(
                "multiple servers exist ({}); pass --server <name>",
                names.join(", ")
            )
            .into())
        }
    }
}
