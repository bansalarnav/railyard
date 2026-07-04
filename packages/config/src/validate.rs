use std::collections::HashMap;
use std::fmt;

use crate::model::{RailyardConfig, Service, Strategy};
use crate::reference::{Reference, parse_references};

#[derive(Debug, Clone)]
pub struct ValidationError {
    /// Where in the config, as a dotted path (`services.api.scale`).
    pub path: String,
    pub message: String,
}

impl ValidationError {
    pub fn new(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path, self.message)
    }
}

pub fn validate(config: &RailyardConfig) -> Vec<ValidationError> {
    let mut errors = Vec::new();

    if let Some(project) = &config.project {
        if !is_valid_name(&project.name) {
            errors.push(ValidationError::new(
                "project.name",
                "must be a lowercase DNS label (a-z, 0-9, hyphens; max 63 chars) — it is used in generated hostnames",
            ));
        }
    }

    if let Some(github) = &config.github {
        check_repo(&mut errors, "github.repo", &github.repo);
    }

    for (name, service) in &config.services {
        validate_service(&mut errors, config, name, service);
    }

    if let Some(cycle) = find_env_reference_cycle(config) {
        errors.push(ValidationError::new(
            "services",
            format!(
                "environment variable references form a cycle: {}",
                cycle.join(" -> ")
            ),
        ));
    }

    errors
}

fn validate_service(
    errors: &mut Vec<ValidationError>,
    config: &RailyardConfig,
    name: &str,
    service: &Service,
) {
    let at = |field: &str| format!("services.{name}.{field}");
    let root = format!("services.{name}");

    if !is_valid_name(name) {
        errors.push(ValidationError::new(
            &root,
            "service names must be lowercase DNS labels (a-z, 0-9, hyphens; max 63 chars) — the name is the internal hostname",
        ));
    }

    let sources = [
        service.path.is_some(),
        service.image.is_some(),
        service.github.is_some(),
    ]
    .iter()
    .filter(|set| **set)
    .count();
    if sources != 1 {
        errors.push(ValidationError::new(
            &root,
            "a service needs exactly one source: `path`, `image`, or `github`",
        ));
    }

    if let Some(path) = &service.path {
        check_relative_path(errors, &at("path"), path);
    }
    if let Some(github) = &service.github {
        check_repo(errors, &at("github.repo"), &github.repo);
        if let Some(path) = &github.path {
            check_relative_path(errors, &at("github.path"), path);
        }
    }
    if service.build.is_some() && service.image.is_some() {
        errors.push(ValidationError::new(
            at("build"),
            "`build` does not apply to `image` services — there is nothing to build",
        ));
    }
    for path in &service.env_files {
        check_relative_path(errors, &at("envFiles"), path);
    }

    if service.is_public() && service.port.is_none() {
        errors.push(ValidationError::new(
            at("port"),
            "`port` is required when a service is public — the proxy needs an upstream",
        ));
    }
    if service.port == Some(0) {
        errors.push(ValidationError::new(at("port"), "port cannot be 0"));
    }

    if let Some(healthcheck) = &service.healthcheck {
        if !healthcheck.path.starts_with('/') {
            errors.push(ValidationError::new(
                at("healthcheck.path"),
                "must start with `/`",
            ));
        }
        if healthcheck.timeout == Some(0) || healthcheck.interval == Some(0) {
            errors.push(ValidationError::new(
                at("healthcheck"),
                "`timeout` and `interval` must be at least 1 second",
            ));
        }
    }

    if let Some(cron) = &service.cron {
        if cron.split_whitespace().count() != 5 {
            errors.push(ValidationError::new(
                at("cron"),
                "expected a 5-field cron expression (minute hour day month weekday)",
            ));
        }
    }

    for dependency in &service.depends_on {
        if dependency == name {
            errors.push(ValidationError::new(
                at("dependsOn"),
                "a service cannot depend on itself",
            ));
        } else if !config.services.contains_key(dependency) {
            errors.push(ValidationError::new(
                at("dependsOn"),
                format!("unknown service `{dependency}`"),
            ));
        }
    }

    for mount_path in service.volumes.values() {
        if !mount_path.starts_with('/') {
            errors.push(ValidationError::new(
                at("volumes"),
                format!("mount path `{mount_path}` must be absolute"),
            ));
        }
    }

    if let Some(scale) = &service.scale {
        if scale.replicas.is_some() && scale.autoscale.is_some() {
            errors.push(ValidationError::new(
                at("scale"),
                "`replicas` and `autoscale` are mutually exclusive",
            ));
        }
        if scale.replicas == Some(0) {
            errors.push(ValidationError::new(
                at("scale.replicas"),
                "must be at least 1",
            ));
        }
        if let Some(autoscale) = &scale.autoscale {
            if autoscale.min < 1 || autoscale.max < autoscale.min {
                errors.push(ValidationError::new(
                    at("scale.autoscale"),
                    "`min` must be >= 1 and `max` >= `min`",
                ));
            }
            if let Some(target) = autoscale.target_cpu_percent {
                if !(1..=100).contains(&target) {
                    errors.push(ValidationError::new(
                        at("scale.autoscale.targetCpuPercent"),
                        "must be between 1 and 100",
                    ));
                }
            }
        }
        let scaled_out = scale.replicas.is_some_and(|replicas| replicas > 1)
            || scale.autoscale.as_ref().is_some_and(|a| a.max > 1);
        if scaled_out && !service.volumes.is_empty() {
            errors.push(ValidationError::new(
                at("scale"),
                "services with volumes are pinned to 1 replica — two containers must never share a volume",
            ));
        }
    }

    if service.strategy == Some(Strategy::Rolling) && !service.volumes.is_empty() {
        errors.push(ValidationError::new(
            at("strategy"),
            "services with volumes cannot roll — use `recreate` (or omit `strategy`)",
        ));
    }

    if let Some(resources) = &service.resources {
        if resources.cpu.is_some_and(|cpu| cpu <= 0.0) {
            errors.push(ValidationError::new(
                at("resources.cpu"),
                "must be greater than 0",
            ));
        }
        if let Some(memory) = &resources.memory {
            if !is_valid_memory(memory) {
                errors.push(ValidationError::new(
                    at("resources.memory"),
                    format!("`{memory}` is not a valid size — use forms like `512Mi` or `1Gi`"),
                ));
            }
        }
    }

    for (key, value) in &service.env {
        let path = format!("services.{name}.env.{key}");
        match parse_references(value) {
            Err(invalid) => errors.push(ValidationError::new(&path, invalid.to_string())),
            Ok(references) => {
                for reference in references {
                    check_reference(errors, config, &path, &reference);
                }
            }
        }
    }
}

fn check_reference(
    errors: &mut Vec<ValidationError>,
    config: &RailyardConfig,
    path: &str,
    reference: &Reference,
) {
    let Some(target) = reference.service() else {
        return; // secrets are resolved server-side; nothing to check here
    };
    let Some(service) = config.services.get(target) else {
        errors.push(ValidationError::new(
            path,
            format!("references unknown service `{target}`"),
        ));
        return;
    };
    let needs_port = matches!(
        reference,
        Reference::ServicePort(_) | Reference::ServiceUrl(_)
    );
    if needs_port && service.port.is_none() {
        errors.push(ValidationError::new(
            path,
            format!("`{target}` must declare `port` to be referenced by port/url"),
        ));
    }
}

/// Cycle detection over `services.<x>.env.<KEY>` references: an env value
/// referencing another service's env means that service's env must resolve
/// first. host/port/url references don't create resolution dependencies.
fn find_env_reference_cycle(config: &RailyardConfig) -> Option<Vec<String>> {
    let mut edges: HashMap<&str, Vec<&str>> = HashMap::new();
    for (name, service) in &config.services {
        for value in service.env.values() {
            let Ok(references) = parse_references(value) else {
                continue; // already reported as an invalid reference
            };
            for reference in references {
                if let Reference::ServiceEnv(target, _) = reference {
                    if let Some((target, _)) = config.services.get_key_value(target.as_str()) {
                        edges.entry(name).or_default().push(target);
                    }
                }
            }
        }
    }

    #[derive(Clone, Copy, PartialEq)]
    enum State {
        Visiting,
        Done,
    }

    fn visit<'a>(
        node: &'a str,
        edges: &HashMap<&'a str, Vec<&'a str>>,
        states: &mut HashMap<&'a str, State>,
        stack: &mut Vec<&'a str>,
    ) -> Option<Vec<String>> {
        match states.get(node) {
            Some(State::Done) => return None,
            Some(State::Visiting) => {
                let start = stack.iter().position(|entry| *entry == node).unwrap_or(0);
                let mut cycle: Vec<String> = stack[start..].iter().map(|s| s.to_string()).collect();
                cycle.push(node.to_string());
                return Some(cycle);
            }
            None => {}
        }
        states.insert(node, State::Visiting);
        stack.push(node);
        for next in edges.get(node).into_iter().flatten() {
            if let Some(cycle) = visit(next, edges, states, stack) {
                return Some(cycle);
            }
        }
        stack.pop();
        states.insert(node, State::Done);
        None
    }

    let mut states = HashMap::new();
    let mut stack = Vec::new();
    for node in edges.keys() {
        if let Some(cycle) = visit(node, &edges, &mut states, &mut stack) {
            return Some(cycle);
        }
    }
    None
}

fn is_valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 63
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !name.starts_with('-')
        && !name.ends_with('-')
}

fn is_valid_memory(memory: &str) -> bool {
    for suffix in ["Mi", "Gi", "M", "G"] {
        if let Some(number) = memory.strip_suffix(suffix) {
            return !number.is_empty() && number.chars().all(|c| c.is_ascii_digit());
        }
    }
    false
}

fn check_repo(errors: &mut Vec<ValidationError>, path: &str, repo: &str) {
    let valid = matches!(
        repo.split_once('/'),
        Some((owner, name)) if !owner.is_empty() && !name.is_empty() && !name.contains('/')
    );
    if !valid {
        errors.push(ValidationError::new(
            path,
            format!("`{repo}` is not an `owner/name` GitHub repo"),
        ));
    }
}

/// Paths in the config (service dirs, env files) must stay inside the
/// directory that holds the config file.
fn check_relative_path(errors: &mut Vec<ValidationError>, at: &str, path: &str) {
    if path.starts_with('/') {
        errors.push(ValidationError::new(
            at,
            format!("`{path}` must be relative to the config file"),
        ));
    } else if path.split('/').any(|component| component == "..") {
        errors.push(ValidationError::new(
            at,
            format!("`{path}` must not escape the config file's directory (`..`)"),
        ));
    }
}
