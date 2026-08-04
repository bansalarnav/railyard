use pingora::http::RequestHeader;
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;

use crate::config::Config;

const API_UPSTREAM_NAME: &str = "railyard";

pub(crate) struct RoutingTable {
    api_addr: SocketAddr,
    api_hostname: String,
    service_upstreams: Arc<BTreeMap<String, SocketAddr>>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RouteTarget {
    pub(super) upstream_addr: SocketAddr,
    pub(super) upstream_name: String,
}

impl RoutingTable {
    pub(crate) fn from_config(config: &Config) -> Self {
        Self {
            api_addr: config.api_addr,
            api_hostname: config.api_hostname.clone(),
            service_upstreams: config.service_upstreams.clone(),
        }
    }

    pub(super) fn route_for_request(&self, request: &RequestHeader) -> Option<RouteTarget> {
        let host = request_host(request);
        self.route_for(host.as_deref())
    }
    fn route_for(&self, host: Option<&str>) -> Option<RouteTarget> {
        if host == Some(self.api_hostname.as_str()) {
            return Some(RouteTarget {
                upstream_addr: self.api_addr,
                upstream_name: API_UPSTREAM_NAME.to_string(),
            });
        }

        let service = host?.split('.').next()?;
        let upstream_addr = *self.service_upstreams.get(service)?;
        Some(RouteTarget {
            upstream_addr,
            upstream_name: service.to_string(),
        })
    }
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
            api_hostname: "railyard.example.com".to_string(),
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
            name_of(table().route_for(Some("railyard.example.com"))),
            Some("railyard".to_string())
        );
    }

    #[test]
    fn service_host_routes_to_upstream() {
        assert_eq!(
            name_of(table().route_for(Some("web.example.com"))),
            Some("web".to_string())
        );
    }

    #[test]
    fn unknown_host_has_no_route() {
        let table = table();
        assert_eq!(name_of(table.route_for(Some("other.example.com"))), None);
        assert_eq!(name_of(table.route_for(None)), None);
    }
}
