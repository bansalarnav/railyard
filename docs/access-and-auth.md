# Access and application auth

Railyard owns the reverse proxy in front of every public service. That makes it
possible to offer useful authentication features without requiring every
application to implement them independently.

There are two related but distinct features here:

- **Access policies** protect a service or route at the proxy. The application
  does not need to know about authentication.
- **Application auth** gives applications an identity provider they can
  integrate with using OAuth/OIDC.

These should share an identity source, but they should not be presented as the
same capability. Access is an ingress concern; application auth changes the
application's own user and session model.

## Proxy-enforced access

The clearest initial feature is an `access` field on `public`. It could protect
admin panels, preview environments, dashboards, and third-party containers
which do not provide suitable authentication themselves.

A simple whole-service policy might look like:

```json
{
  "services": {
    "dashboard": {
      "image": "example/dashboard:latest",
      "port": 8080,
      "public": {
        "domain": "dashboard.example.com",
        "access": {
          "require": "authenticated",
          "allowEmails": ["alice@example.com"],
          "allowDomains": ["example.com"]
        }
      }
    }
  }
}
```

Path-specific rules would allow a mostly public application to protect only
routes such as `/admin`:

```json
{
  "services": {
    "web": {
      "path": ".",
      "port": 3000,
      "public": {
        "domain": "example.com",
        "access": [
          {
            "path": "/admin",
            "require": "authenticated",
            "allowEmails": ["alice@example.com"]
          }
        ]
      }
    }
  }
}
```

For an incoming request, the proxy would:

1. Match the domain, service, and path.
2. Select the most specific access rule.
3. Validate the user's session and evaluate the rule.
4. Redirect an unauthenticated browser to login, or return an appropriate
   `401`/`403` response for a non-browser request.
5. Forward an authorized request to the service.

Railyard could optionally pass verified identity to the upstream:

```text
x-railyard-user-id: usr_...
x-railyard-user-email: alice@example.com
```

The proxy must always remove any incoming copies of these headers before
inserting its own. Applications should only trust them when they cannot be
reached except through the Railyard proxy.

### Initial policy model

The first policy language should remain intentionally small:

- `require: authenticated`
- `allowEmails`
- `allowDomains`
- possibly `allowOrganizations` for providers which expose a reliable
  organization claim

If an `access` policy exists, failure to satisfy it should deny access. For
multiple path policies, the most specific matching path should win. A general
RBAC system or arbitrary policy expressions can wait until concrete use cases
justify them.

Access policies belong under `public`, rather than directly on a service,
because they govern ingress behavior. Internal service-to-service traffic does
not pass through the proxy and should not accidentally acquire different
semantics.

## Top-level auth configuration

A top-level `auth` field can define the identity source used by access
policies. Initially, this could point at an existing OIDC issuer:

```json
{
  "auth": {
    "issuer": "https://auth.example.com",
    "clientId": "railyard",
    "clientSecret": "${{ secrets.AUTH_CLIENT_SECRET }}"
  }
}
```

This gives Railyard a narrow responsibility: run the authorization-code flow,
establish its own browser session, and enforce ingress policies. It does not
make Railyard the application's user database.

A later managed form could reduce setup further:

```json
{
  "auth": {
    "managed": true,
    "providers": ["github"]
  }
}
```

The managed form would deploy or embed an authorization server and configure
Railyard's proxy as one of its clients. Provider credentials should still be
stored as Railyard secrets; GitHub, Google, and similar providers require the
operator to create an OAuth application and configure its callback URL, so the
experience cannot be literally zero-config.

## OpenAuth

OpenAuth is a plausible implementation for managed auth because it is
self-hosted, standards-based, supports common upstream providers, and can run
as a standalone service. It should be kept behind a Railyard-owned
configuration model rather than exposing OpenAuth-specific concepts directly
in the manifest. That leaves room to replace it or support other issuers later.

OpenAuth is an authorization server, not the whole proxy-access feature.
Railyard would still own:

- callback and login routing;
- secure browser sessions and cookies;
- OAuth state, PKCE, and CSRF protections;
- access-policy evaluation;
- logout, session expiry, revocation, and signing-key rotation;
- stripping and injecting trusted upstream headers.

OpenAuth currently describes itself as beta, so it should be evaluated behind
this abstraction rather than becoming part of Railyard's public contract.

## Keep deployment auth separate

Railyard already authenticates CLI users with signed requests and device
keys. That system controls who can deploy and administer projects. It should
remain separate from the identities of people visiting deployed applications.

The two systems may eventually integrate—for example, an access rule meaning
"any Railyard member of this project"—but the current CLI identity has no
browser login mechanism, email identity, or browser session. Combining them
prematurely would blur two different security boundaries.

## Suggested rollout

1. Define the access-policy model and proxy enforcement behavior.
2. Support one external OIDC issuer as the identity source.
3. Add proxy-managed sessions and trusted upstream identity headers.
4. Add Railyard-managed auth, potentially powered by OpenAuth.
5. Later, expose the managed issuer directly to applications which want native
   OAuth/OIDC integration.

The first three steps already produce a distinctive feature: Cloudflare
Access-style route protection expressed in the same manifest that deploys the
service. Full application auth can grow from that foundation without being
required for the initial value.

## Questions to resolve

- Is `access` allowed only on the whole public route initially, or do we begin
  with path-specific rules?
- Are sessions scoped to one project, one domain, or the whole Railyard server?
- What is the behavior for APIs: redirect, `401`, or configurable handling?
- Which identity claims are stable enough to authorize against?
- Should access rules support machine credentials in addition to browser
  users?
- Should non-production environments be protectable by default with a single
  project-level setting?
