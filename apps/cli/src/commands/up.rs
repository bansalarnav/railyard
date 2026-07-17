use flate2::Compression;
use flate2::write::GzEncoder;
use ignore::WalkBuilder;
use std::error::Error;
use std::io::{self, Write};
use std::path::Path;
use std::{env, fs};

use crate::context::ExecContext;
use crate::resolve::{
    LinkedProject, MANIFEST_FILE, ProjectPresence, confirm_ancestor, find_manifest, parse_manifest,
    resolve_project_server, resolve_server, server_project_presence,
};

/// Extra ignore rules for the upload, in gitignore syntax — for things that
/// belong in the repo but not in a deploy snapshot.
const RAILYARD_IGNORE_FILE: &str = ".railyardignore";

/// Validate the manifest and pack the repository for deploy
#[derive(clap::Args)]
pub(crate) struct Args {
    #[arg(long)]
    server: Option<String>,
}

pub(crate) async fn run(args: Args, ctx: ExecContext) -> Result<(), Box<dyn Error>> {
    let cwd = env::current_dir()?;
    let Some((manifest_path, raw)) = find_manifest(&cwd)? else {
        return Err(format!(
            "no {MANIFEST_FILE} found here or in any ancestor; run `railyard init` first"
        )
        .into());
    };
    let root = manifest_path
        .parent()
        .ok_or("manifest has no parent directory")?
        .to_path_buf();
    let manifest = parse_manifest(&manifest_path, &raw)
        .map_err(|error| format!("{} is invalid:\n{error}", manifest_path.display()))?;

    // `up` never invents projects: a manifest without an id points at `init`
    // instead of silently creating something on whatever server resolves.
    let Some(project) = manifest.project.as_ref().and_then(|project| {
        project.id.as_ref().map(|id| LinkedProject {
            id: id.clone(),
            name: project.name.clone(),
            manifest_path: (root != cwd).then(|| manifest_path.clone()),
        })
    }) else {
        return Err(format!(
            "{} has no project.id; run `railyard init` to create the project on a server",
            manifest_path.display()
        )
        .into());
    };
    if !confirm_ancestor(&project, ctx)? {
        return Err("up cancelled".into());
    }

    let (server_name, server) = match args.server {
        Some(name) => resolve_server(Some(name))?,
        None => resolve_project_server(&project, ctx).await?,
    };

    // The project must already exist on the resolved server; creating it here
    // would silently fork the project onto the wrong box.
    match server_project_presence(&server, &project).await {
        ProjectPresence::Present => {}
        ProjectPresence::Absent => {
            return Err(format!(
                "server {server_name} ({}) does not have project {} ({}); check --server or \
                 run `railyard init`",
                server.server_url, project.name, project.id
            )
            .into());
        }
        ProjectPresence::Unknown(reason) => {
            return Err(format!(
                "could not confirm project {} on {server_name}: {reason}",
                project.name
            )
            .into());
        }
    }

    println!(
        "Packing {} for project {} on {server_name} ({})",
        root.display(),
        project.name,
        server.server_url
    );
    let archive_path = env::temp_dir().join(format!("railyard-up-{}.tar.gz", project.id));
    let files = pack_repository(&root, &archive_path)?;
    println!(
        "Packed {files} files into {} ({})",
        archive_path.display(),
        human_size(fs::metadata(&archive_path)?.len())
    );
    println!("Upload is not implemented yet; the archive was left in place.");
    Ok(())
}

/// Gzipped tarball of the repository at `root`, honoring `.gitignore` and
/// `.railyardignore` and always skipping `.git` and `node_modules`. Paths in
/// the archive are relative to `root`. Returns the number of files packed.
fn pack_repository(root: &Path, output: &Path) -> Result<u64, Box<dyn Error>> {
    let encoder = GzEncoder::new(
        io::BufWriter::new(fs::File::create(output)?),
        Compression::default(),
    );
    let mut archive = tar::Builder::new(encoder);
    archive.follow_symlinks(false);

    let mut builder = WalkBuilder::new(root);
    let output = output.to_path_buf();
    builder
        // Hidden files ship (.railyard.json itself, dotfile configs); the
        // ignore rules decide what stays out, not the leading dot.
        .hidden(false)
        // Honor .gitignore even when the tree isn't a git checkout.
        .require_git(false)
        .add_custom_ignore_filename(RAILYARD_IGNORE_FILE)
        .filter_entry(move |entry| {
            entry.file_name() != ".git"
                && entry.file_name() != "node_modules"
                && entry.path() != output
        })
        .sort_by_file_name(|a, b| a.cmp(b));

    let mut files = 0;
    for entry in builder.build() {
        let entry = entry?;
        let relative = entry.path().strip_prefix(root)?;
        if relative.as_os_str().is_empty() {
            continue;
        }
        archive.append_path_with_name(entry.path(), relative)?;
        if !entry.file_type().is_some_and(|kind| kind.is_dir()) {
            files += 1;
        }
    }

    archive.into_inner()?.finish()?.flush()?;
    Ok(files)
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KiB", "MiB", "GiB", "TiB"];
    let mut size = bytes as f64;
    let mut unit = None;
    for next in UNITS {
        if size < 1024.0 {
            break;
        }
        size /= 1024.0;
        unit = Some(next);
    }
    match unit {
        Some(unit) => format!("{size:.1} {unit}"),
        None => format!("{bytes} B"),
    }
}
