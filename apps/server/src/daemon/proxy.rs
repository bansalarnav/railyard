use async_trait::async_trait;
use pingora::Result;
use pingora::http::RequestHeader;
use pingora::proxy::{ProxyHttp, Session};
use pingora::upstreams::peer::HttpPeer;
use std::net::SocketAddr;

use super::state::AppState;

#[derive(Clone)]
pub(crate) struct RoutingTable {
    pub(crate) api_addr: SocketAddr,
}

#[derive(Clone, Debug)]
pub(crate) struct RouteTarget {
    pub(crate) upstream_addr: SocketAddr,
    pub(crate) upstream_name: String,
}

pub(crate) struct ControlPlaneProxy {
    pub(crate) routes: RoutingTable,
}

impl RoutingTable {
    pub(crate) fn from_state(state: &AppState) -> Self {
        Self {
            api_addr: state.api_addr,
        }
    }

    fn route_for_request(&self, _request: &RequestHeader) -> RouteTarget {
        RouteTarget {
            upstream_addr: self.api_addr,
            upstream_name: "api".to_string(),
        }
    }
}

#[async_trait]
impl ProxyHttp for ControlPlaneProxy {
    type CTX = RouteTarget;

    fn new_ctx(&self) -> Self::CTX {
        RouteTarget {
            upstream_addr: self.routes.api_addr,
            upstream_name: "api".to_string(),
        }
    }

    async fn upstream_peer(
        &self,
        session: &mut Session,
        ctx: &mut Self::CTX,
    ) -> Result<Box<HttpPeer>> {
        *ctx = self.routes.route_for_request(session.req_header());
        Ok(Box::new(HttpPeer::new(
            ctx.upstream_addr,
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
        upstream_request.insert_header("x-railyard-upstream", ctx.upstream_name.as_str())?;
        Ok(())
    }
}
