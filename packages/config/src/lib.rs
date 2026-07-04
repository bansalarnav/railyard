//! Parsing, validation, and rewriting of `.railyard.json`.
//!
//! Shared by the CLI (parse before upload, write ids back during
//! `railyard new`/`link`) and the server (parse uploads and pushed commits).
//! See docs/config.md for the format itself.
//!
//! ```
//! let config = railyard_config::parse(input)?;
//! let staging = config.resolve_environment("staging")?;
//! ```

mod model;
mod reference;
mod validate;

use std::fmt;

pub use model::*;
pub use reference::{InvalidReference, Reference, parse_references};
pub use validate::ValidationError;

#[derive(Debug)]
pub enum ConfigError {
    /// Not JSON at all; carries serde's line/column message.
    Syntax(serde_json::Error),
    /// JSON, but the wrong shape (unknown field, wrong type, ...).
    /// `path` points at the offending key, e.g. `services.api.scale.replicas`.
    Shape { path: String, message: String },
    /// Well-shaped, but semantically wrong. Every problem found, not just the first.
    Invalid(Vec<ValidationError>),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Syntax(err) => write!(f, "invalid JSON: {err}"),
            ConfigError::Shape { path, message } if path == "." => write!(f, "{message}"),
            ConfigError::Shape { path, message } => write!(f, "{path}: {message}"),
            ConfigError::Invalid(errors) => {
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

impl std::error::Error for ConfigError {}

/// Parse and fully validate a `.railyard.json` string, including checking
/// that every `environments` overlay resolves to a valid config.
pub fn parse(input: &str) -> Result<RailyardConfig, ConfigError> {
    let raw: serde_json::Value = serde_json::from_str(input).map_err(ConfigError::Syntax)?;
    let config = parse_value(raw)?;

    let mut errors = Vec::new();
    for name in config.environments.keys() {
        match config.resolve_environment(name) {
            Ok(_) => {}
            Err(ConfigError::Invalid(env_errors)) => {
                errors.extend(env_errors.into_iter().map(|error| {
                    ValidationError::new(
                        format!("environments.{name}.{}", error.path),
                        error.message,
                    )
                }));
            }
            Err(ConfigError::Shape { path, message }) => {
                errors.push(ValidationError::new(
                    format!("environments.{name}.{path}"),
                    message,
                ));
            }
            Err(ConfigError::Syntax(err)) => {
                errors.push(ValidationError::new(
                    format!("environments.{name}"),
                    err.to_string(),
                ));
            }
        }
    }
    if !errors.is_empty() {
        return Err(ConfigError::Invalid(errors));
    }
    Ok(config)
}

/// Deserialize + validate one concrete config (environment overlays are not
/// followed here — `parse` handles those).
fn parse_value(raw: serde_json::Value) -> Result<RailyardConfig, ConfigError> {
    let config: RailyardConfig =
        serde_path_to_error::deserialize(raw).map_err(|err| ConfigError::Shape {
            path: err.path().to_string(),
            message: err.inner().to_string(),
        })?;
    let errors = validate::validate(&config);
    if !errors.is_empty() {
        return Err(ConfigError::Invalid(errors));
    }
    Ok(config)
}

impl RailyardConfig {
    /// The config with one `environments` overlay deep-merged in (objects
    /// merge, `null` deletes a key, everything else replaces). The result has
    /// no `environments` of its own and is re-validated as a whole.
    pub fn resolve_environment(&self, name: &str) -> Result<RailyardConfig, ConfigError> {
        let overlay = self.environments.get(name).ok_or_else(|| {
            ConfigError::Invalid(vec![ValidationError::new(
                "environments",
                format!("no environment named `{name}`"),
            )])
        })?;
        if !overlay.is_object() {
            return Err(ConfigError::Invalid(vec![ValidationError::new(
                format!("environments.{name}"),
                "an environment overlay must be an object",
            )]));
        }

        let mut base = serde_json::to_value(self).map_err(ConfigError::Syntax)?;
        base.as_object_mut()
            .expect("config always serializes to an object")
            .remove("environments");
        deep_merge(&mut base, overlay);
        parse_value(base)
    }

    /// Serialize back to the canonical on-disk form: 2-space pretty JSON,
    /// original key order (maps preserve insertion order), trailing newline.
    pub fn to_json_string(&self) -> String {
        let mut out = serde_json::to_string_pretty(self).expect("config always serializes cleanly");
        out.push('\n');
        out
    }

    /// Record the server-assigned project id (`railyard new` / `railyard link`).
    pub fn link_project(&mut self, name: &str, id: &str) {
        match &mut self.project {
            Some(project) => project.id = Some(id.to_string()),
            None => {
                self.project = Some(Project {
                    id: Some(id.to_string()),
                    name: name.to_string(),
                });
            }
        }
    }

    /// Record the repo link written by `railyard github link`.
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
