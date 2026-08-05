mod model;
mod reference;
mod validate;

use anyhow::{Result, anyhow, bail};

pub use model::*;
pub use reference::{Reference, parse_references};
pub use validate::ValidationError;

pub fn parse(input: &str) -> Result<RailyardManifest> {
    let raw: serde_json::Value =
        serde_json::from_str(input).map_err(|err| anyhow!("invalid JSON: {err}"))?;
    parse_raw(raw)
}

/// `parse` for the relaxed spellings — JSONC and JSON5 (a superset of
/// JSONC), so comments and trailing commas are fine.
pub fn parse_relaxed(input: &str) -> Result<RailyardManifest> {
    let raw: serde_json::Value =
        json5::from_str(input).map_err(|err| anyhow!("invalid JSON: {err}"))?;
    parse_raw(raw)
}

fn parse_raw(raw: serde_json::Value) -> Result<RailyardManifest> {
    parse_value(raw)
}
fn parse_value(raw: serde_json::Value) -> Result<RailyardManifest> {
    let manifest: RailyardManifest = serde_path_to_error::deserialize(raw).map_err(|err| {
        let path = err.path().to_string();
        let message = err.inner();
        if path == "." {
            anyhow!("{message}")
        } else {
            anyhow!("{path}: {message}")
        }
    })?;
    let errors = validate::validate(&manifest);
    if !errors.is_empty() {
        bail!(
            "{}",
            errors
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
    Ok(manifest)
}

impl RailyardManifest {
    pub fn to_json_string(&self) -> String {
        let mut out =
            serde_json::to_string_pretty(self).expect("manifest always serializes cleanly");
        out.push('\n');
        out
    }
    pub fn link_project(&mut self, name: &str, id: &str) {
        match &mut self.project {
            Some(project) => {
                project.name = name.to_string();
                project.id = Some(id.to_string());
            }
            None => {
                self.project = Some(Project {
                    id: Some(id.to_string()),
                    name: name.to_string(),
                });
            }
        }
    }
    pub fn link_github(&mut self, repo: &str, branch: Option<&str>) {
        self.github = Some(GithubLink {
            repo: repo.to_string(),
            branch: branch.map(str::to_string),
        });
    }
}
