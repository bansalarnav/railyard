use hyper::Uri;
use pingora::http::RequestHeader;
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;

use crate::config::Config;

const API_LABEL: &str = "railyard";
const API_PATH_PREFIX: &str = "/railyard";

pub(crate) struct RoutingTable {
    api_addr: SocketAddr,
    service_upstreams: Arc<BTreeMap<String, SocketAddr>>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RouteTarget {
    pub(super) upstream_addr: SocketAddr,
    pub(super) upstream_name: String,
    /// Set when the API was reached through `/railyard`: that prefix is a
    /// mount point, not part of the API's own paths, so it is stripped before
    /// the request is forwarded.
    pub(super) strip_api_prefix: bool,
}

impl RoutingTable {
    pub(crate) fn from_config(config: &Config) -> Self {
        Self {
            api_addr: config.api_addr,
            service_upstreams: config.service_upstreams.clone(),
        }
    }

    pub(super) fn route_for_request(&self, request: &RequestHeader) -> Option<RouteTarget> {
        let host = request_host(request);
        self.route_for(host.as_deref(), request.uri.path())
    }
    fn route_for(&self, host: Option<&str>, path: &str) -> Option<RouteTarget> {
        let host_label = host.and_then(|host| host.split('.').next());

        if host_label == Some(API_LABEL) || is_api_path(path) {
            return Some(RouteTarget {
                upstream_addr: self.api_addr,
                upstream_name: API_LABEL.to_string(),
                strip_api_prefix: is_api_path(path),
            });
        }

        let service = host_label?;
        let upstream_addr = *self.service_upstreams.get(service)?;
        Some(RouteTarget {
            upstream_addr,
            upstream_name: service.to_string(),
            strip_api_prefix: false,
        })
    }
}

fn is_api_path(path: &str) -> bool {
    path.strip_prefix(API_PATH_PREFIX)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

/// Drop the `/railyard` mount point so the API sees its own paths. The client
/// signs the path relative to that mount, so this is what the signature
/// covers. `/railyard` itself becomes `/`.
pub(super) fn strip_api_prefix(uri: &Uri) -> Option<Uri> {
    let path_and_query = uri.path_and_query()?.as_str();
    let rest = path_and_query.strip_prefix(API_PATH_PREFIX)?;
    let rest = if rest.is_empty() || rest.starts_with('?') {
        format!("/{rest}")
    } else {
        rest.to_string()
    };
    rest.parse().ok()
}

fn request_host(request: &RequestHeader) -> Option<String> {
    let host = match request.uri.host() {
        Some(host) => host,
        None => request.headers.get("host")?.to_str().ok()?,
    };
    let host = host.split(':').next().unwrap_or(host);
    Some(host.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> RoutingTable {
        RoutingTable {
            api_addr: "127.0.0.1:3001".parse().unwrap(),
            service_upstreams: Arc::new(BTreeMap::from([(
                "web".to_string(),
                "127.0.0.1:4000".parse().unwrap(),
            )])),
        }
    }

    fn name_of(target: Option<RouteTarget>) -> Option<String> {
        target.map(|target| target.upstream_name)
    }

    #[test]
    fn railyard_host_routes_to_api() {
        assert_eq!(
            name_of(table().route_for(Some("railyard.example.com"), "/anything")),
            Some("railyard".to_string())
        );
    }

    #[test]
    fn railyard_path_routes_to_api() {
        let table = table();
        assert_eq!(
            name_of(table.route_for(Some("example.com"), "/railyard")),
            Some("railyard".to_string())
        );
        assert_eq!(
            name_of(table.route_for(Some("example.com"), "/railyard/api/users")),
            Some("railyard".to_string())
        );
        assert_eq!(
            name_of(table.route_for(Some("example.com"), "/railyardx")),
            None
        );
    }

    #[test]
    fn service_host_routes_to_upstream() {
        assert_eq!(
            name_of(table().route_for(Some("web.example.com"), "/")),
            Some("web".to_string())
        );
    }

    #[test]
    fn unknown_host_has_no_route() {
        let table = table();
        assert_eq!(
            name_of(table.route_for(Some("other.example.com"), "/")),
            None
        );
        assert_eq!(name_of(table.route_for(None, "/")), None);
    }
}
