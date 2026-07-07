use async_trait::async_trait;
use pingora::Result;
use pingora::http::RequestHeader;
use pingora::proxy::{ProxyHttp, Session};
use pingora::upstreams::peer::HttpPeer;
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;

use super::state::AppState;

const API_LABEL: &str = "railyard";
const API_PATH_PREFIX: &str = "/railyard";

pub(crate) struct RoutingTable {
    api_addr: SocketAddr,
    service_upstreams: Arc<BTreeMap<String, SocketAddr>>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RouteTarget {
    upstream_addr: SocketAddr,
    upstream_name: String,
}

impl RoutingTable {
    pub(crate) fn from_state(state: &AppState) -> Self {
        Self {
            api_addr: state.api_addr,
            service_upstreams: state.service_upstreams.clone(),
        }
    }

    fn route_for_request(&self, request: &RequestHeader) -> Option<RouteTarget> {
        let host = request_host(request);
        self.route_for(host.as_deref(), request.uri.path())
    }
    fn route_for(&self, host: Option<&str>, path: &str) -> Option<RouteTarget> {
        let host_label = host.and_then(|host| host.split('.').next());

        if host_label == Some(API_LABEL) || is_api_path(path) {
            return Some(RouteTarget {
                upstream_addr: self.api_addr,
                upstream_name: API_LABEL.to_string(),
            });
        }

        let service = host_label?;
        let upstream_addr = *self.service_upstreams.get(service)?;
        Some(RouteTarget {
            upstream_addr,
            upstream_name: service.to_string(),
        })
    }
}

pub(crate) struct IngressProxy {
    pub(crate) routes: RoutingTable,
}

#[async_trait]
impl ProxyHttp for IngressProxy {
    type CTX = Option<RouteTarget>;

    fn new_ctx(&self) -> Self::CTX {
        None
    }

    async fn request_filter(&self, session: &mut Session, ctx: &mut Self::CTX) -> Result<bool> {
        *ctx = self.routes.route_for_request(session.req_header());
        if ctx.is_none() {
            session.respond_error(404).await?;
            return Ok(true);
        }
        Ok(false)
    }

    async fn upstream_peer(
        &self,
        _session: &mut Session,
        ctx: &mut Self::CTX,
    ) -> Result<Box<HttpPeer>> {
        let target = ctx.as_ref().expect("route was decided in request_filter");
        Ok(Box::new(HttpPeer::new(
            target.upstream_addr,
            false,
            String::new(),
        )))
    }

    async fn upstream_request_filter(
        &self,
        _session: &mut Session,
        upstream_request: &mut RequestHeader,
        ctx: &mut Self::CTX,
    ) -> Result<()> {
        if let Some(target) = ctx {
            upstream_request.insert_header("x-railyard-upstream", target.upstream_name.as_str())?;
        }
        Ok(())
    }
}

fn is_api_path(path: &str) -> bool {
    path.strip_prefix(API_PATH_PREFIX)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
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
            name_of(table.route_for(Some("example.com"), "/railyard/api/services")),
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
