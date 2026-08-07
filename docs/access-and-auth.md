# Managed auth and proxy access

There are two independent product ideas here:

- **Managed auth** gives deployed applications an auth system they can
  integrate with, without the operator having to deploy and maintain a
  separate auth stack.
- **Proxy access** protects a public service or route at Railyard's reverse
  proxy, without requiring the application to implement authentication.

Neither feature should depend on the other. They may eventually reuse some
identity or protocol machinery, but they solve different problems and should
not be coupled in the manifest.

## Managed application auth

Managed auth would be the self-hosted equivalent of adding Clerk, Auth0, or
Supabase Auth to an application, except that Railyard provisions and operates
it alongside the project.

The application deliberately integrates with this auth system. It receives an
authenticated identity, creates or validates sessions, and decides what that
user may do inside the application. Railyard removes the operational work; it
does not try to infer the application's authorization model at the proxy.

The product promise could be:

> Deploy your application and add self-hosted authentication from the same
> manifest, without operating a separate auth stack.

### Manifest shape

`auth` should be a top-level project field because it is shared infrastructure,
not a property of one container. A configuration might begin like this:

```json
{
  "auth": {
    "providers": {
      "github": {
        "clientId": "${{ secrets.GITHUB_CLIENT_ID }}",
        "clientSecret": "${{ secrets.GITHUB_CLIENT_SECRET }}"
      },
      "email": true
    },
    "clients": {
      "web": {
        "service": "web",
        "redirectPaths": ["/auth/callback"]
      }
    }
  },
  "services": {
    "web": {
      "path": ".",
      "port": 3000,
      "public": true
    }
  }
}
```

The exact shape can become smaller as the defaults become clear. For example,
Railyard may be able to infer the client name, redirect URL, and public origin
from the referenced service. Provider credentials still need to come from
secrets: GitHub, Google, and similar providers require the operator to create
an OAuth application and register its callback URL.

`auth` is a better manifest name than `managedAuth`. "Managed auth" describes
the product capability; the manifest should keep the common concept concise.

### What Railyard manages

Enabling `auth` should provision a logical auth tenant for the project and
manage:

- a stable issuer URL;
- user and credential storage;
- signing keys and rotation;
- OAuth/OIDC endpoints and callbacks;
- access and refresh tokens;
- hosted login, signup, logout, and account-recovery screens;
- upstream providers such as GitHub or Google;
- registered clients for selected services;
- schema migrations, upgrades, and backups;
- generated client credentials and secret injection;
- user administration through the Railyard CLI.

The implementation may use one shared auth runtime with an isolated logical
tenant per project, or a separate runtime per project. The public model should
promise project isolation rather than expose that implementation choice.

Railyard should automatically provide selected services with conventional
environment variables:

```text
RAILYARD_AUTH_ISSUER=https://auth-myproject.example.com
RAILYARD_AUTH_CLIENT_ID=...
RAILYARD_AUTH_CLIENT_SECRET=...
```

Explicit references may also be useful when an application expects different
variable names:

```json
{
  "env": {
    "AUTH_ISSUER": "${{ auth.issuer }}",
    "AUTH_CLIENT_ID": "${{ auth.clients.web.id }}",
    "AUTH_CLIENT_SECRET": "${{ auth.clients.web.secret }}"
  }
}
```

Generated client secrets must remain server-side secrets. They should never be
written into the normalized manifest returned to the client or committed to
the repository.

### What the application still does

Managed auth cannot make application integration disappear entirely. The
application still needs to:

- start and complete the login flow;
- validate tokens or establish its own session;
- associate the stable auth subject with application data;
- protect application routes and actions;
- implement application concepts such as teams, roles, and permissions.

Railyard can make this small with hosted UI, standards-compatible OAuth/OIDC,
clear examples, and optional framework adapters. Standards support is
important: an application should not need a Railyard-specific SDK if its
existing auth library can consume an issuer URL and client credentials.

### OpenAuth

OpenAuth is a plausible engine for managed auth because it is self-hosted,
standards-based, supports common upstream providers, and can run as a
standalone service.

It should remain an implementation detail behind a Railyard-owned manifest
and lifecycle. Railyard would still need to provide:

- deployment and supervision of the auth runtime;
- durable storage and backup behavior;
- project tenant creation and deletion;
- provider and client configuration;
- stable domains, routing, and TLS;
- generated secret storage and injection;
- hosted UI defaults and customization boundaries;
- user-management commands;
- upgrades and migrations.

OpenAuth currently describes itself as beta, so it should be evaluated behind
this abstraction rather than becoming part of Railyard's public contract. If
it is replaced later, applications using standard OAuth/OIDC should not need
to change their Railyard manifests.

### Keep deployment auth separate

Railyard already authenticates CLI users with signed requests and device keys.
That controls who may deploy and administer projects. Managed application auth
identifies the users of a deployed application. These are separate security
boundaries and separate user populations.

They may eventually integrate—for example, the CLI could administer the
application's auth users—but a Railyard deployer must not automatically become
an application user, and application credentials must never authorize control
plane operations.

### Suggested managed-auth rollout

1. Define the project auth tenant, client, secret, and deletion lifecycle.
2. Provision one managed issuer with one login method and hosted login UI.
3. Generate a client for a selected service and inject its configuration.
4. Add CLI user administration, recovery, key rotation, upgrades, and backup
   behavior.
5. Add more providers and framework-specific integration packages as demand
   becomes concrete.

A deliberately narrow first version—one project, one web client, and one login
method—would validate whether the experience is actually easier before
Railyard grows a broad auth platform.

### Managed-auth questions

- What is the lowest-friction first login method: GitHub, email magic links,
  or passkeys?
- What durable store owns auth data, and how is it backed up and restored?
- Does deleting `auth` destroy its users, retain them, or require an explicit
  destructive command?
- Is there one stable issuer per project across all environments, or one per
  environment?
- How are preview-environment redirect URLs registered safely?
- Which hosted pages are included, and how much branding is configurable?
- Does the first version issue browser sessions, tokens, or both?
- How are generated client credentials rotated without breaking deployments?

## Proxy-enforced access

Proxy access is a separate feature made possible because Railyard owns the
reverse proxy in front of every public service. It is useful for admin panels,
preview environments, dashboards, and third-party containers which do not
provide suitable authentication themselves.

An `access` field belongs on `public` because it governs ingress behavior, not
the container or internal service-to-service traffic:

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

Path-specific rules could allow a mostly public application to protect only
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
3. Validate the visitor's access session and evaluate the rule.
4. Redirect an unauthenticated browser to login, or return an appropriate
   `401`/`403` response for a non-browser request.
5. Forward an authorized request to the service.

Railyard could optionally pass verified identity to the upstream:

```text
x-railyard-user-id: usr_...
x-railyard-user-email: alice@example.com
```

The proxy must always remove incoming copies of these headers before inserting
its own. Applications should only trust them when they cannot be reached
except through the Railyard proxy.

The initial policy language should remain intentionally small:

- `require: authenticated`
- `allowEmails`
- `allowDomains`
- possibly `allowOrganizations` for providers which expose a reliable
  organization claim

If an `access` policy exists, failure to satisfy it should deny access. For
multiple path policies, the most specific matching path should win. General
RBAC or arbitrary policy expressions can wait for concrete use cases.

Proxy access needs some mechanism for authenticating visitors, but that does
not mean it depends on the project's managed application auth. Its identity
source and sessions should be designed as part of the access feature. A future
adapter could allow managed auth to be one supported source without coupling
the two manifest models.

### Proxy-access questions

- Is `access` allowed only on the whole public route initially, or do we begin
  with path-specific rules?
- What identity sources does proxy access support first?
- Are access sessions scoped to one project, one domain, or the whole server?
- What is the behavior for APIs: redirect, `401`, or configurable handling?
- Which identity claims are stable enough to authorize against?
- Should access rules support machine credentials in addition to browser
  users?
- Should non-production environments be protectable by default with a single
  project-level setting?
