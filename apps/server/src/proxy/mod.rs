//! The ingress proxy: the one port the outside world talks to. It decides
//! which upstream a request belongs to and forwards it there. Everything it
//! serves — including the Railyard API — is an upstream behind this.

mod routing;

use async_trait::async_trait;
use pingora::Result;
use pingora::http::RequestHeader;
use pingora::proxy::{ProxyHttp, Session};
use pingora::upstreams::peer::HttpPeer;

use routing::RouteTarget;

pub(crate) use routing::RoutingTable;

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
