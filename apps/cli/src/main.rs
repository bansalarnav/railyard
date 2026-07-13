mod auth;
mod config;
mod http;

use clap::{Parser, Subcommand};
use dialoguer::{Confirm, Input, Select, theme::ColorfulTheme};
use railyard_auth::{INVITE_BLOB_PREFIX, InvitePayload, WhoamiResponse, unix_timestamp};
use railyard_manifest::RailyardManifest;
use std::error::Error;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs, io};

use auth::{generate_signing_key, public_key_base64};
use config::{
    ServerConfig, list_servers, read_project_binding, read_server, rebind_projects,
    record_project_binding, remove_project_binding, remove_server, sanitize_server_name,
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
        /// An invite blob, or an SSH target (user@host) to mint one on
        target: String,
        /// Local name for this server; defaults to the name embedded in the invite
        #[arg(long)]
        name: Option<String>,
        /// User to create when logging in over SSH; defaults to your local username
        #[arg(long)]
        user: Option<String>,
    },
    /// Show every identity this machine holds and which one commands here would use
    Whoami {
        /// Only check this server
        #[arg(long)]
        server: Option<String>,
    },
    /// Create a project on a server and link this directory to it
    Init {
        /// Project name; otherwise prompts when creating a manifest
        name: Option<String>,
        #[arg(long)]
        server: Option<String>,
    },
    /// Pick one of your servers and link this directory's project to it
    Link,
    /// Forget which server this directory's project is linked to
    Unlink,
    User {
        #[command(subcommand)]
        command: UserCommand,
    },
}

#[derive(Subcommand)]
enum UserCommand {
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

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Login { target, name, user } => {
            if target.starts_with(INVITE_BLOB_PREFIX) {
                login(&target, name)
            } else {
                login_ssh(&target, name, user)
            }
        }
        Commands::Whoami { server } => whoami(server),
        Commands::Init { name, server } => init(name, server),
        Commands::Link => link(),
        Commands::Unlink => unlink(),
        Commands::User { command } => match command {
            UserCommand::Add {
                name,
                admin,
                server,
            } => user_add(&name, admin, server),
            UserCommand::List { server } => user_list(server),
            UserCommand::Remove { name, server } => user_remove(&name, server),
        },
    }
}

/// One row per server entry, queried live so the answer reflects what the
/// server believes (a revoked key shows up here, not in local config). The
/// starred row is what commands in the current directory would use, computed
/// with the same resolution rules those commands apply.
fn whoami(server_flag: Option<String>) -> Result<(), Box<dyn Error>> {
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
        let via = match &project.manifest_dir {
            Some(dir) => format!(", manifest in {}", dir.display()),
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
        let who = match http::whoami(server) {
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

/// Invite to the project this directory is linked to; `--server` only pins
/// which server entry to use, like every other project command. `--admin`
/// switches to a server-wide (admin) invite. Either way the server only
/// honors the request from an admin key.
fn user_add(name: &str, admin: bool, server_flag: Option<String>) -> Result<(), Box<dyn Error>> {
    if admin {
        return user_add_admin(name, server_flag);
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
            return user_add_admin(name, None);
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

fn user_add_admin(name: &str, server_flag: Option<String>) -> Result<(), Box<dyn Error>> {
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

fn user_list(server_flag: Option<String>) -> Result<(), Box<dyn Error>> {
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

fn user_remove(name: &str, server_flag: Option<String>) -> Result<(), Box<dyn Error>> {
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

struct LinkedProject {
    id: String,
    name: String,
    /// Set when the manifest was found in an ancestor directory, not here.
    manifest_dir: Option<PathBuf>,
}

/// The project this directory belongs to, if any: the nearest
/// `.railyard.json` here or in an ancestor, carrying a `project.id` (both
/// written by `railyard init`).
fn linked_project() -> Result<Option<LinkedProject>, Box<dyn Error>> {
    let cwd = env::current_dir()?;
    let Some((dir, raw)) = find_manifest(&cwd)? else {
        return Ok(None);
    };

    let manifest = railyard_manifest::parse(&raw)?;
    let manifest_dir = (dir != cwd).then_some(dir);
    Ok(manifest.project.and_then(|project| {
        project.id.map(|id| LinkedProject {
            id,
            name: project.name,
            manifest_dir,
        })
    }))
}

/// The nearest manifest at or above `start`. The walk stops at the first
/// file found — a manifest without a `project.id` still ends the search.
fn find_manifest(start: &Path) -> Result<Option<(PathBuf, String)>, Box<dyn Error>> {
    let mut dir = start.to_path_buf();
    loop {
        match fs::read_to_string(dir.join(MANIFEST_FILE)) {
            Ok(raw) => return Ok(Some((dir, raw))),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        if !dir.pop() {
            return Ok(None);
        }
    }
}

/// `linked_project`, gated when the manifest lives in an ancestor directory:
/// confirm before acting on it, so a stray subdirectory never silently
/// targets the parent's project. Report-only callers (`whoami`) use
/// `linked_project` directly.
fn confirmed_linked_project() -> Result<Option<LinkedProject>, Box<dyn Error>> {
    let Some(project) = linked_project()? else {
        return Ok(None);
    };
    let Some(dir) = &project.manifest_dir else {
        return Ok(Some(project));
    };

    if !io::stdin().is_terminal() {
        return Err(format!(
            "no {MANIFEST_FILE} in this directory, but {} has one (project {}); rerun from \
             there to use it",
            dir.display(),
            project.name
        )
        .into());
    }
    let confirmed = Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt(format!(
            "Use project {} from {}?",
            project.name,
            dir.join(MANIFEST_FILE).display()
        ))
        .default(true)
        .interact()?;
    Ok(confirmed.then_some(project))
}

/// The server for a linked project: the recorded binding, or — when none
/// exists (or the bound entry is gone) — an offer to link a server that
/// already has the project.
fn resolve_project_server(
    project: &LinkedProject,
) -> Result<(String, ServerConfig), Box<dyn Error>> {
    match project_binding(&project.id)? {
        ProjectBinding::Bound(name, server) => Ok((name, server)),
        ProjectBinding::Stale(stale) => {
            eprintln!(
                "note: project {} was linked to {stale}, which no longer exists on this \
                 machine; looking for the project on your other servers",
                project.name
            );
            offer_project_link(project)
        }
        ProjectBinding::Unbound => offer_project_link(project),
    }
}

enum ProjectBinding {
    Bound(String, ServerConfig),
    /// A binding exists but its server entry is gone (removed config); the
    /// name is kept for reporting.
    Stale(String),
    Unbound,
}

/// The binding recorded by `init` or a project-scoped `login`, checked
/// against the server entries that still exist.
fn project_binding(project_id: &str) -> Result<ProjectBinding, Box<dyn Error>> {
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
fn offer_project_link(project: &LinkedProject) -> Result<(String, ServerConfig), Box<dyn Error>> {
    let mut candidates: Vec<(String, ServerConfig)> = Vec::new();
    let mut unchecked: Vec<String> = Vec::new();
    for (name, server) in list_servers()? {
        match server_project_presence(&server, project) {
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
            if !io::stdin().is_terminal() {
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
            if !io::stdin().is_terminal() {
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

fn link_project(
    project: &LinkedProject,
    name: String,
    server: ServerConfig,
) -> Result<(String, ServerConfig), Box<dyn Error>> {
    record_project_binding(&project.id, &name)?;
    println!("Linked project {} ({}) to {name}", project.name, project.id);
    Ok((name, server))
}

enum ProjectPresence {
    Present,
    Absent,
    /// Couldn't tell — the server was unreachable, the key rejected, or the
    /// listing failed. The reason is kept for reporting.
    Unknown(String),
}

/// Could this identity act on the project, and does its server have it?
fn server_project_presence(server: &ServerConfig, project: &LinkedProject) -> ProjectPresence {
    match http::whoami(server) {
        Ok(http::WhoamiOutcome::Identity(identity)) => match identity.project_id {
            Some(scoped) if scoped == project.id => ProjectPresence::Present,
            Some(_) => ProjectPresence::Absent,
            None => match http::list_projects(server) {
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

fn init(name: Option<String>, server_flag: Option<String>) -> Result<(), Box<dyn Error>> {
    let manifest_path = Path::new(MANIFEST_FILE);
    let (mut manifest, manifest_exists) = match fs::read_to_string(manifest_path) {
        Ok(raw) => (railyard_manifest::parse(&raw)?, true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            confirm_nested_init()?;
            (RailyardManifest::default(), false)
        }
        Err(error) => return Err(error.into()),
    };

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

    let project_name = resolve_project_name(name, &manifest, manifest_exists)?;
    let (server_name, server) = resolve_server_for_init(server_flag)?;

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
            if !io::stdin().is_terminal() {
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
                fs::write(manifest_path, manifest.to_json_string())?;
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
    fs::write(manifest_path, manifest.to_json_string())?;
    record_project_binding(&created.id, &server_name)?;

    println!(
        "Created project {} ({}) and linked {MANIFEST_FILE}",
        created.name, created.id
    );
    Ok(())
}

/// Scaffolding a manifest inside an existing project's tree is almost
/// always `init` run from the wrong directory, so ask before creating a
/// nested project.
fn confirm_nested_init() -> Result<(), Box<dyn Error>> {
    let cwd = env::current_dir()?;
    let Some(parent) = cwd.parent() else {
        return Ok(());
    };
    let Some((dir, raw)) = find_manifest(parent)? else {
        return Ok(());
    };
    let found = dir.join(MANIFEST_FILE);
    // A broken ancestor manifest shouldn't block init here; name the file
    // and let the user decide.
    let project = railyard_manifest::parse(&raw)
        .ok()
        .and_then(|manifest| manifest.project)
        .map(|project| format!(" (project {})", project.name))
        .unwrap_or_default();

    if !io::stdin().is_terminal() {
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

/// Show every server this machine knows and link this directory's project to
/// the chosen one. Unlike the automatic offer in `offer_project_link`, the
/// list is not narrowed to servers that have the project — the choice is
/// checked after, so picking a server without it points at `init` instead of
/// silently recording a bad binding.
fn link() -> Result<(), Box<dyn Error>> {
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
        ProjectBinding::Stale(stale) => eprintln!(
            "note: ignoring the link to {stale}, which no longer exists on this machine"
        ),
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

/// Drop the recorded project→server binding. The manifest keeps its
/// `project.id`, so `init` (or any project command) can link it again — to
/// the same server or another one.
fn unlink() -> Result<(), Box<dyn Error>> {
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

/// Explicit arg wins unless the manifest already names a different project.
/// A new manifest prompts on a TTY, defaulting to the directory name squeezed
/// into the same DNS-label shape that `project.name` validation demands.
fn resolve_project_name(
    explicit: Option<String>,
    manifest: &RailyardManifest,
    manifest_exists: bool,
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
    if !manifest_exists && io::stdin().is_terminal() {
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

/// `--server` flag, then the sole known server. Never prompts — commands
/// other than `init` must be told which server when several exist.
fn resolve_server(explicit: Option<String>) -> Result<(String, ServerConfig), Box<dyn Error>> {
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

/// `init` is where a server gets chosen for a project, so it alone may
/// prompt: with several servers and no `--server`, show a picker on a TTY.
fn resolve_server_for_init(
    explicit: Option<String>,
) -> Result<(String, ServerConfig), Box<dyn Error>> {
    let mut servers = list_servers()?;
    if explicit.is_some() || servers.len() < 2 || !io::stdin().is_terminal() {
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
            && (identity.project_id.is_none()
                || identity.project_id.as_deref() == Some(project_id))
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
