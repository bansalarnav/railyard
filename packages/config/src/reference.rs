use std::fmt;

/// A `${{ ... }}` reference inside an env value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reference {
    /// `${{ services.<name>.host }}` — the internal hostname (the name itself).
    ServiceHost(String),
    /// `${{ services.<name>.port }}` — the declared port.
    ServicePort(String),
    /// `${{ services.<name>.url }}` — `http://<name>:<port>`.
    ServiceUrl(String),
    /// `${{ services.<name>.env.<KEY> }}` — a variable shared from another service.
    ServiceEnv(String, String),
    /// `${{ secrets.<KEY> }}` — a server-stored secret.
    Secret(String),
}

impl Reference {
    /// The service this reference points at, if any.
    pub fn service(&self) -> Option<&str> {
        match self {
            Reference::ServiceHost(name)
            | Reference::ServicePort(name)
            | Reference::ServiceUrl(name)
            | Reference::ServiceEnv(name, _) => Some(name),
            Reference::Secret(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidReference {
    pub token: String,
    pub reason: String,
}

impl fmt::Display for InvalidReference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid reference `{}`: {}", self.token, self.reason)
    }
}

/// Extract every `${{ ... }}` reference from an env value. Text outside the
/// double braces (including single `$VAR`) is left alone by design — the
/// syntax exists so dotenv-style values never collide with it.
pub fn parse_references(value: &str) -> Result<Vec<Reference>, InvalidReference> {
    let mut references = Vec::new();
    let mut rest = value;

    while let Some(start) = rest.find("${{") {
        let after_open = &rest[start + 3..];
        let Some(end) = after_open.find("}}") else {
            return Err(InvalidReference {
                token: rest[start..].to_string(),
                reason: "unterminated `${{` (missing `}}`)".to_string(),
            });
        };
        let token = after_open[..end].trim();
        references.push(parse_token(token)?);
        rest = &after_open[end + 2..];
    }

    Ok(references)
}

fn parse_token(token: &str) -> Result<Reference, InvalidReference> {
    let invalid = |reason: &str| InvalidReference {
        token: token.to_string(),
        reason: reason.to_string(),
    };

    if let Some(key) = token.strip_prefix("secrets.") {
        if key.is_empty() {
            return Err(invalid("missing secret name"));
        }
        return Ok(Reference::Secret(key.to_string()));
    }

    if let Some(rest) = token.strip_prefix("services.") {
        let Some((name, attr)) = rest.split_once('.') else {
            return Err(invalid(
                "expected `services.<name>.host|port|url` or `services.<name>.env.<KEY>`",
            ));
        };
        if name.is_empty() {
            return Err(invalid("missing service name"));
        }
        return match attr {
            "host" => Ok(Reference::ServiceHost(name.to_string())),
            "port" => Ok(Reference::ServicePort(name.to_string())),
            "url" => Ok(Reference::ServiceUrl(name.to_string())),
            _ => match attr.strip_prefix("env.") {
                Some(key) if !key.is_empty() => {
                    Ok(Reference::ServiceEnv(name.to_string(), key.to_string()))
                }
                Some(_) => Err(invalid("missing variable name after `env.`")),
                None => Err(invalid(
                    "expected `host`, `port`, `url`, or `env.<KEY>` after the service name",
                )),
            },
        };
    }

    Err(invalid(
        "references must start with `services.` or `secrets.`",
    ))
}
