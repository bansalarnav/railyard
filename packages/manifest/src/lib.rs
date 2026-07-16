mod model;
mod reference;
mod validate;

use std::fmt;

pub use model::*;
pub use reference::{InvalidReference, Reference, parse_references};
pub use validate::ValidationError;

#[derive(Debug)]
pub enum ManifestError {
    Syntax(String),
    Shape { path: String, message: String },
    Invalid(Vec<ValidationError>),
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ManifestError::Syntax(err) => write!(f, "invalid JSON: {err}"),
            ManifestError::Shape { path, message } if path == "." => write!(f, "{message}"),
            ManifestError::Shape { path, message } => write!(f, "{path}: {message}"),
            ManifestError::Invalid(errors) => {
                for (i, error) in errors.iter().enumerate() {
                    if i > 0 {
                        writeln!(f)?;
                    }
                    write!(f, "{error}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for ManifestError {}
pub fn parse(input: &str) -> Result<RailyardManifest, ManifestError> {
    let raw: serde_json::Value =
        serde_json::from_str(input).map_err(|err| ManifestError::Syntax(err.to_string()))?;
    parse_raw(raw)
}

/// `parse` for the relaxed spellings — JSONC and JSON5 (a superset of
/// JSONC), so comments and trailing commas are fine.
pub fn parse_relaxed(input: &str) -> Result<RailyardManifest, ManifestError> {
    let raw: serde_json::Value =
        json5::from_str(input).map_err(|err| ManifestError::Syntax(err.to_string()))?;
    parse_raw(raw)
}

fn parse_raw(raw: serde_json::Value) -> Result<RailyardManifest, ManifestError> {
    let manifest = parse_value(raw)?;

    let mut errors = Vec::new();
    for name in manifest.environments.keys() {
        match manifest.resolve_environment(name) {
            Ok(_) => {}
            Err(ManifestError::Invalid(env_errors)) => {
                errors.extend(env_errors.into_iter().map(|error| {
                    ValidationError::new(
                        format!("environments.{name}.{}", error.path),
                        error.message,
                    )
                }));
            }
            Err(ManifestError::Shape { path, message }) => {
                errors.push(ValidationError::new(
                    format!("environments.{name}.{path}"),
                    message,
                ));
            }
            Err(ManifestError::Syntax(message)) => {
                errors.push(ValidationError::new(format!("environments.{name}"), message));
            }
        }
    }
    if !errors.is_empty() {
        return Err(ManifestError::Invalid(errors));
    }
    Ok(manifest)
}
fn parse_value(raw: serde_json::Value) -> Result<RailyardManifest, ManifestError> {
    let manifest: RailyardManifest =
        serde_path_to_error::deserialize(raw).map_err(|err| ManifestError::Shape {
            path: err.path().to_string(),
            message: err.inner().to_string(),
        })?;
    let errors = validate::validate(&manifest);
    if !errors.is_empty() {
        return Err(ManifestError::Invalid(errors));
    }
    Ok(manifest)
}

impl RailyardManifest {
    pub fn resolve_environment(&self, name: &str) -> Result<RailyardManifest, ManifestError> {
        let overlay = self.environments.get(name).ok_or_else(|| {
            ManifestError::Invalid(vec![ValidationError::new(
                "environments",
                format!("no environment named `{name}`"),
            )])
        })?;
        if !overlay.is_object() {
            return Err(ManifestError::Invalid(vec![ValidationError::new(
                format!("environments.{name}"),
                "an environment overlay must be an object",
            )]));
        }

        let mut base = serde_json::to_value(self)
            .map_err(|err| ManifestError::Syntax(err.to_string()))?;
        base.as_object_mut()
            .expect("manifest always serializes to an object")
            .remove("environments");
        deep_merge(&mut base, overlay);
        parse_value(base)
    }
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

fn deep_merge(base: &mut serde_json::Value, overlay: &serde_json::Value) {
    match (base, overlay) {
        (serde_json::Value::Object(base), serde_json::Value::Object(overlay)) => {
            for (key, value) in overlay {
                if value.is_null() {
                    base.remove(key);
                } else {
                    deep_merge(
                        base.entry(key.clone()).or_insert(serde_json::Value::Null),
                        value,
                    );
                }
            }
        }
        (base, overlay) => *base = overlay.clone(),
    }
}
