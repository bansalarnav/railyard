use indexmap::IndexMap;
use serde::{Deserialize, Deserializer, Serialize};
use serde_with::skip_serializing_none;

pub const DEFAULT_HEALTHCHECK_TIMEOUT: u64 = 60;
pub const DEFAULT_HEALTHCHECK_INTERVAL: u64 = 3;
pub const DEFAULT_DRAIN_SECONDS: u64 = 30;
pub const DEFAULT_AUTOSCALE_TARGET_CPU: u32 = 70;

/// The parsed shape of a `.railyard.json` file.
///
/// `environments` overlays are kept as raw JSON and applied with
/// [`RailyardManifest::resolve_environment`](crate::RailyardManifest::resolve_environment),
/// so the base manifest stays one concrete set of types instead of a parallel
/// all-optional copy of every struct.
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct RailyardManifest {
    #[serde(rename = "$schema")]
    pub schema: Option<String>,
    pub project: Option<Project>,
    pub github: Option<GithubLink>,
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    pub services: IndexMap<String, Service>,
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    pub environments: IndexMap<String, serde_json::Value>,
}

impl Default for RailyardManifest {
    fn default() -> Self {
        Self {
            schema: None,
            project: None,
            github: None,
            services: IndexMap::new(),
            environments: IndexMap::new(),
        }
    }
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Project {
    /// Written by `railyard new` / `railyard link`; absent in a hand-written
    /// file that has not been linked to a server project yet.
    pub id: Option<String>,
    pub name: String,
}

/// Project-level GitHub link: the repo that contains the manifest file.
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct GithubLink {
    /// `owner/name`.
    pub repo: String,
    pub branch: Option<String>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct Service {
    pub path: Option<String>,
    pub image: Option<String>,
    pub github: Option<ServiceGithub>,
    pub build: Option<Build>,
    pub start: Option<String>,
    pub port: Option<u16>,
    #[serde(
        deserialize_with = "de_env_map",
        skip_serializing_if = "IndexMap::is_empty"
    )]
    pub env: IndexMap<String, String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub env_files: Vec<String>,
    pub public: Option<Public>,
    pub healthcheck: Option<Healthcheck>,
    pub restart: Option<RestartPolicy>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
    pub cron: Option<String>,
    pub strategy: Option<Strategy>,
    pub drain: Option<u64>,
    /// volume name -> absolute mount path inside the container.
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    pub volumes: IndexMap<String, String>,
    pub resources: Option<Resources>,
    pub scale: Option<Scale>,
}

impl Default for Service {
    fn default() -> Self {
        Self {
            path: None,
            image: None,
            github: None,
            build: None,
            start: None,
            port: None,
            env: IndexMap::new(),
            env_files: Vec::new(),
            public: None,
            healthcheck: None,
            restart: None,
            depends_on: Vec::new(),
            cron: None,
            strategy: None,
            drain: None,
            volumes: IndexMap::new(),
            resources: None,
            scale: None,
        }
    }
}

/// Exactly one of `path` / `image` / `github` must be set on a service.
#[derive(Debug, Clone, Copy)]
pub enum Source<'a> {
    Path(&'a str),
    Image(&'a str),
    Github(&'a ServiceGithub),
}

impl Service {
    /// The service's source, if exactly one is declared. Validation rejects
    /// zero or multiple sources, so on a validated manifest this is always Some.
    pub fn source(&self) -> Option<Source<'_>> {
        match (&self.path, &self.image, &self.github) {
            (Some(p), None, None) => Some(Source::Path(p)),
            (None, Some(i), None) => Some(Source::Image(i)),
            (None, None, Some(g)) => Some(Source::Github(g)),
            _ => None,
        }
    }

    pub fn is_public(&self) -> bool {
        self.public.as_ref().is_some_and(Public::is_enabled)
    }

    pub fn restart(&self) -> RestartPolicy {
        self.restart.unwrap_or(RestartPolicy::OnFailure)
    }

    /// `rolling` unless overridden — but volumes always force `recreate`,
    /// since two containers must never share a volume.
    pub fn effective_strategy(&self) -> Strategy {
        if !self.volumes.is_empty() {
            Strategy::Recreate
        } else {
            self.strategy.unwrap_or(Strategy::Rolling)
        }
    }

    pub fn drain(&self) -> u64 {
        self.drain.unwrap_or(DEFAULT_DRAIN_SECONDS)
    }
}

/// Service source in a different GitHub repo.
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ServiceGithub {
    /// `owner/name`.
    pub repo: String,
    pub branch: Option<String>,
    /// Subdirectory within that repo to build from.
    pub path: Option<String>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct Build {
    pub dockerfile: Option<String>,
    #[serde(skip_serializing_if = "IndexMap::is_empty")]
    pub args: IndexMap<String, String>,
    pub watch: Option<Vec<String>>,
}

/// `"public": true` or `"public": { "domain": ..., ... }`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Public {
    Toggle(bool),
    Options(PublicOptions),
}

impl Public {
    pub fn is_enabled(&self) -> bool {
        match self {
            Public::Toggle(enabled) => *enabled,
            Public::Options(_) => true,
        }
    }

    /// Custom domains, merging the `domain` and `domains` spellings.
    /// Empty means "auto subdomain".
    pub fn domains(&self) -> Vec<&str> {
        match self {
            Public::Toggle(_) => Vec::new(),
            Public::Options(options) => options
                .domain
                .as_deref()
                .into_iter()
                .chain(options.domains.iter().map(String::as_str))
                .collect(),
        }
    }
}

#[skip_serializing_none]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct PublicOptions {
    pub domain: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub domains: Vec<String>,
    /// Path prefix for path-based routing on a shared domain.
    pub path: Option<String>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Healthcheck {
    /// HTTP path polled on the service's `port`.
    pub path: String,
    pub timeout: Option<u64>,
    pub interval: Option<u64>,
}

impl Healthcheck {
    pub fn timeout(&self) -> u64 {
        self.timeout.unwrap_or(DEFAULT_HEALTHCHECK_TIMEOUT)
    }

    pub fn interval(&self) -> u64 {
        self.interval.unwrap_or(DEFAULT_HEALTHCHECK_INTERVAL)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RestartPolicy {
    Always,
    OnFailure,
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Strategy {
    Rolling,
    Recreate,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct Resources {
    /// Cores, fractional allowed.
    pub cpu: Option<f64>,
    /// `512Mi`, `1Gi`, ...
    pub memory: Option<String>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct Scale {
    pub replicas: Option<u32>,
    pub autoscale: Option<Autoscale>,
}

#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Autoscale {
    pub min: u32,
    pub max: u32,
    pub target_cpu_percent: Option<u32>,
}

impl Autoscale {
    pub fn target_cpu_percent(&self) -> u32 {
        self.target_cpu_percent
            .unwrap_or(DEFAULT_AUTOSCALE_TARGET_CPU)
    }
}

/// Accept strings, numbers, and booleans as env values (`"PORT": 3000` is a
/// mistake nobody should have to debug), normalizing everything to strings.
fn de_env_map<'de, D>(deserializer: D) -> Result<IndexMap<String, String>, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::Error;

    let raw = IndexMap::<String, serde_json::Value>::deserialize(deserializer)?;
    let mut env = IndexMap::with_capacity(raw.len());
    for (key, value) in raw {
        let value = match value {
            serde_json::Value::String(s) => s,
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::Bool(b) => b.to_string(),
            other => {
                return Err(D::Error::custom(format!(
                    "env value for `{key}` must be a string, number, or boolean, got {other}"
                )));
            }
        };
        env.insert(key, value);
    }
    Ok(env)
}
