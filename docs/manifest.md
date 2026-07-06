# .railyard.json

One file at the repo root defines the whole project. `railyard up` reads it, diffs it against
what the server is running, and syncs. Config in the file always wins over server-side state.

## Full example

```json
{
  "$schema": "https://railyard.dev/schema.json",
  "project": {
    "id": "prj_9f3k2m1xq8",
    "name": "acme"
  },
  "github": {
    "repo": "acme/acme",
    "branch": "main"
  },
  "services": {
    "web": {
      "path": "apps/web",
      "port": 3000,
      "env": {
        "API_URL": "${{ services.api.url }}"
      },
      "public": {
        "domains": ["acme.dev", "www.acme.dev"]
      },
      "scale": { "replicas": 2 }
    },
    "api": {
      "path": "apps/api",
      "port": 8080,
      "build": {
        "dockerfile": "Dockerfile",
        "args": { "NODE_ENV": "production" }
      },
      "start": "node dist/index.js",
      "envFiles": [".env.api"],
      "env": {
        "DATABASE_URL": "postgres://acme:${{ secrets.POSTGRES_PASSWORD }}@${{ services.db.host }}:5432/acme",
        "REDIS_URL": "redis://${{ services.cache.host }}:6379",
        "STRIPE_SECRET_KEY": "${{ secrets.STRIPE_SECRET_KEY }}"
      },
      "healthcheck": { "path": "/healthz", "timeout": 30 },
      "public": { "domain": "api.acme.dev" },
      "scale": {
        "autoscale": { "min": 1, "max": 4, "targetCpuPercent": 70 }
      },
      "resources": { "cpu": 1, "memory": "1Gi" },
      "dependsOn": ["db", "cache"]
    },
    "worker": {
      "path": "apps/api",
      "start": "node dist/worker.js",
      "envFiles": [".env.api"],
      "dependsOn": ["db", "cache"]
    },
    "db": {
      "image": "postgres:16",
      "port": 5432,
      "env": {
        "POSTGRES_USER": "acme",
        "POSTGRES_PASSWORD": "${{ secrets.POSTGRES_PASSWORD }}",
        "POSTGRES_DB": "acme"
      },
      "volumes": { "pgdata": "/var/lib/postgresql/data" }
    },
    "cache": {
      "image": "redis:7"
    },
    "docs": {
      "github": { "repo": "acme/docs-site", "branch": "main", "path": "site" },
      "port": 4321,
      "public": { "domain": "docs.acme.dev" }
    },
    "nightly-backup": {
      "image": "ghcr.io/acme/backup:latest",
      "cron": "0 3 * * *",
      "env": { "DATABASE_URL": "${{ services.api.env.DATABASE_URL }}" }
    }
  },
  "environments": {
    "staging": {
      "services": {
        "web": { "public": { "domain": "staging.acme.dev" }, "scale": { "replicas": 1 } },
        "api": { "scale": { "replicas": 1 } }
      }
    }
  }
}
```

## Top level

| Key | Type | Notes |
| --- | --- | --- |
| `$schema` | string | JSON schema for editor autocomplete/validation. |
| `project` | object | `id` (written by `railyard new`/`railyard link`, links the file to a project on the server) and `name` (display name, also used in generated hostnames). |
| `github` | object | Optional. Links the repo containing this file; pushes to `branch` auto-deploy (see [GitHub deploys](#github-deploys)). `repo` (`owner/name`), `branch` (default: repo's default branch). |
| `services` | object | Map of service name → service config. Names are `[a-z0-9-]`, must be unique; the name **is** the internal hostname. |
| `environments` | object | Optional per-environment overrides, deep-merged over the base config (Railway-style). Selected with `railyard up --env staging`; default environment is `production`. |

## Service source (exactly one required)

| Key | Type | Notes |
| --- | --- | --- |
| `path` | string | Directory relative to the config file (same level or deeper — paths outside the repo root are rejected). Built on the server from an uploaded snapshot of that directory (or from the pushed commit when GitHub-deployed). |
| `image` | string | A pullable image reference (`postgres:16`, `ghcr.io/acme/thing:v2`). No build step. |
| `github` | object | A *different* GitHub repo as the source: `repo` (`owner/name`), `branch`, optional `path` (subdirectory within that repo). Pushes to it redeploy just this service. |

`path`, `image`, and `github` are mutually exclusive; a service must have exactly one. Two
services may share a `path` with different `start` commands (e.g. `api` + `worker`).

## Build & runtime (path services)

| Key | Type | Default | Notes |
| --- | --- | --- | --- |
| `build.dockerfile` | string | auto-detect | Dockerfile path relative to `path`. If absent and no Dockerfile is found, fall back to a buildpack (nixpacks/railpack) later; v1 can require a Dockerfile. |
| `build.args` | object | — | Docker build args. |
| `build.watch` | string[] | `["**"]` | Only redeploy this service when matching files changed (useful in monorepos). |
| `start` | string | image CMD | Override the container start command. |

## Networking

Every service in a project joins a private Docker network automatically — **zero config**.
A service is reachable from its siblings at `http://<service-name>:<port>`. Nothing is
reachable from the internet unless it declares `public`.

| Key | Type | Notes |
| --- | --- | --- |
| `port` | number | The port the app listens on. Required if the service is `public` or referenced via `${{ services.x.url }}`; optional otherwise (image `EXPOSE` used as fallback). |
| `public` | `true` \| object | Omitted = internal only. `true` = auto subdomain `<service>-<project>.<server-base-domain>`. Object form: `domain` or `domains` (custom domains pointed at the VPS; the server proxy terminates TLS and routes by Host header), optional `path` prefix for path-based routing on a shared domain. |

### How internal networking works

Each project gets one private bridge network (`ry-<project-id>`). Every container is attached
with a **network alias equal to its service name**, so the runtime's embedded DNS resolves
`api` to the container IP for anyone on the same network. `${{ services.api.host }}` therefore
expands to literally the string `api` — the server only validates that the referenced service
exists. This is exactly docker-compose's naming behavior, so converted apps keep working.

- **Replicas** all share the alias; DNS returns every IP round-robin, giving crude internal
  load balancing for free. (Caveat: runtimes that cache DNS may pin to one replica.)
- **No host ports.** Containers have their own IPs on the project network, so two services can
  both listen on 5432 without conflict, and nothing is ever published with `-p`. The reverse
  proxy reaches upstreams over the project network directly. The only host ports railyard owns
  are 80/443 for the proxy.
- **Cross-project isolation** is automatic: aliases are scoped to a network, and each project
  has its own.
- **`PORT` injection:** railyard sets `PORT=<declared port>` in every container, so apps that
  follow the `$PORT` convention automatically agree with the declared `port`. Apps must listen
  on `0.0.0.0`, not localhost. Image services (postgres, redis) ignore the extra var harmlessly.

## Environment variables

Three layers, merged in order (later wins): `envFiles` → `env` → server-side secrets.

| Key | Type | Notes |
| --- | --- | --- |
| `envFiles` | string[] | Paths to dotenv files relative to the config file. Read **client-side** at `railyard up` time and pushed to the server, so gitignored `.env` files work naturally. |
| `env` | object | Inline `KEY: value` map for non-secret config. Values support `${{ … }}` references. |

### References

`${{ … }}` is resolved by the server at deploy time (double braces so it never collides with
shell `$VAR` syntax inside dotenv files):

- `${{ services.<name>.host }}` — internal hostname (just `<name>`).
- `${{ services.<name>.port }}` — that service's declared port.
- `${{ services.<name>.url }}` — `http://<name>:<port>` shorthand.
- `${{ services.<name>.env.<KEY> }}` — share a variable across services without repeating it.
- `${{ secrets.<KEY> }}` — server-stored secret, set once via `railyard secrets set KEY=value`
  (per project, optionally per environment with `--env`). Secrets never live in the file, so
  the file is always safe to commit.

Cycles in references are a sync-time error.

## Lifecycle & health

| Key | Type | Default | Notes |
| --- | --- | --- | --- |
| `healthcheck.path` | string | — | HTTP path polled on `port`. New deploys only receive traffic once healthy (enables zero-downtime rollouts). |
| `healthcheck.timeout` | number | 60 | Total seconds to wait for healthy before the rollout is rolled back. |
| `healthcheck.interval` | number | 3 | Seconds between polls. |
| `restart` | string | `on-failure` | `always` \| `on-failure` \| `never`. Enforced by the railyard daemon itself (not the container runtime), so it behaves identically on Docker and Podman. |
| `dependsOn` | string[] | — | Start ordering within a sync; waits for the dependency's healthcheck if it has one. |
| `cron` | string | — | Cron expression. The service becomes a job: run on schedule, exit, not part of the long-running set. |
| `strategy` | string | auto | `rolling` \| `recreate`. Defaults to `rolling`; services with `volumes` are forced to `recreate` (two containers must never share a volume). |
| `drain` | number | 30 | Seconds between removing an old container from routing/DNS and sending it SIGTERM, then a grace period before SIGKILL. |

## Zero-downtime rollouts

The invariant: **the old container is never touched until its replacement is verified healthy.**
A rollout of service `api` proceeds:

1. **Start** the new container on the project network, *without* the `api` alias yet.
2. **Verify.** The railyard daemon polls the healthcheck over the project network:
   HTTP `GET healthcheck.path` if declared; otherwise a TCP connect to `port`; otherwise
   just "container still running after `drain` seconds". If the budget (`healthcheck.timeout`)
   expires unhealthy, the new container is killed and the old one keeps serving — a failed
   deploy can never cause downtime.
3. **Switch public traffic.** The proxy is in-process, so its upstream table swaps atomically:
   the next request goes to the new container, in-flight requests to the old one complete.
4. **Switch internal traffic.** Add the `api` alias to the new container, remove it from the
   old one. DNS moves are not atomic — peers with a cached lookup or a keepalive connection
   may keep hitting the old container briefly, which is why it keeps running through the
   `drain` window and only then gets SIGTERM → grace → SIGKILL.
5. **Repeat per replica** (`rolling` = one at a time, so peak overhead is one extra container).
   With `replicas: 1`, rolling degenerates to blue-green: brief 2× resource usage for that
   service, no downtime.

Notes:

- **Volume-backed services can't do this.** Postgres with two containers on one volume is
  corruption, so `volumes` forces `strategy: recreate` — stop old, start new, a few seconds of
  downtime. Be honest about it in output: databases restart, stateless services roll.
- **Public traffic drains perfectly** (the proxy tracks in-flight requests); internal traffic
  drains best-effort via the `drain` window. If that's ever insufficient, the escape hatch is
  routing internal traffic through the proxy too — deliberately not v1.
- **Rollback is cheap**: the previous image stays on disk, so `railyard rollback` is just a
  rollout in the other direction.
- **Migrations**: a future `preDeploy` command slots in between build and step 1 (Railway's
  `preDeployCommand`), running to completion before any traffic moves.

## State & scaling

| Key | Type | Notes |
| --- | --- | --- |
| `volumes` | object | `volume-name: /mount/path`. Named volumes persist across deploys; a service with volumes is pinned to `replicas: 1`. |
| `resources` | object | `cpu` (cores, fractional ok) and `memory` (`512Mi`, `1Gi`) limits per replica. |
| `scale.replicas` | number | Fixed replica count (default 1). Requests to the service are load-balanced across replicas. |
| `scale.autoscale` | object | `{ min, max, targetCpuPercent }`. Mutually exclusive with `replicas`. Single-VPS autoscaling is just container-count scaling, so v1 can accept the key and treat `min` as the count until the scaler exists. |

## CLI lifecycle

```
railyard new        # create project on server, write .railyard.json with project.id,
                    #   scaffold services by scanning for Dockerfiles / docker-compose.yml
railyard link       # adopt an existing server project into an existing/new file (writes project.id)
railyard up         # validate, upload changed path-service snapshots, diff, sync
railyard up --prune # also delete server-side services no longer in the file (destructive, so opt-in)
railyard secrets set KEY=value [--env staging]
railyard github link [owner/repo]   # webhook + deploy key + write github block
railyard status     # per-service state, replicas, domains
```

`railyard new` flow: verify auth against the server → `POST /projects` → receive `prj_…` id →
if `docker-compose.yml` exists offer to convert it; else scan subdirectories for Dockerfiles and
prefill `path` services → write the file → print `railyard up` as the next step. If the file
already has a `project.id`, `new` refuses and points at `link`.

`up` is declarative: file present on server but identical → no-op; changed → redeploy; new →
create. Removal requires `--prune` so a typo'd rename can't take down a database.

## GitHub deploys

Two levels of linking, one mechanism:

- **Project-level `github`** links the repo that contains `.railyard.json` (the common,
  monorepo case). A push to the linked branch makes the server fetch that commit, read
  `.railyard.json` *from the commit* (the pushed file is the source of truth — equivalent to
  running `railyard up` at that commit, never `--prune`), and redeploy only the path services
  whose files changed, using `build.watch` patterns against the git diff. `image` services are
  untouched by pushes.
- **Service-level `github` source** points one service at a different repo (e.g. a docs site
  maintained elsewhere). A push there rebuilds just that service from its repo; the config
  file in *this* repo still owns its settings (env, public, scale).

Deploys triggered by push go through the same rollout machinery as `railyard up` — healthcheck
gate, traffic switch, drain. Pushes to unlinked branches are ignored in v1; branch → environment
mapping later rides the existing `environments` override (e.g.
`environments.staging.github.branch: "develop"`).

### Wiring (self-hosted, no GitHub App required)

`railyard github link` does the setup using the developer's local `gh`/OAuth credentials:

1. Creates a webhook on the repo pointing at `https://<server>/hooks/github/<project-id>`
   (rides the existing proxy on 443) with a generated HMAC secret; the server verifies
   `X-Hub-Signature-256` on every delivery.
2. For private repos, installs a read-only **deploy key** (per repo, generated server-side) so
   the server can fetch commits. Public repos skip this.
3. Writes the `github` block into the config.

A GitHub App (like Railway/Vercel use) is the polished version — commit statuses, PR preview
environments, one-click install — but requires each self-hoster to register their own app.
Deploy key + webhook needs nothing but repo admin, so it's the right v1; the App flow can be
added later via GitHub's app-manifest handoff (the Coolify approach). Escape hatch for servers
that can't receive webhooks: run `railyard up` from a GitHub Action.

The server reports deploy results back as **commit statuses** when credentials allow it
(deploy keys can't; this arrives with the App flow). Until then, `railyard status` and logs
are the feedback loop.

## docker-compose support

Compose is supported as an **on-ramp, not a second config format**. Accepting
`docker-compose.yml` directly would mean running an unpredictable subset of the compose spec
(profiles, anchors, `extends`, bind mounts, host-published `ports` — which contradict the
proxy model) while still needing railyard-only concepts (domains, secrets, autoscale, cron,
project id) bolted on via `x-railyard` extensions. Two formats also means two drifting sources
of truth.

Instead, `railyard new` converts a detected compose file. The mapping is mechanical:

| compose | .railyard.json |
| --- | --- |
| `build.context` / `dockerfile` / `args` | `path` / `build.dockerfile` / `build.args` |
| `image`, `command` | `image`, `start` |
| `environment`, `env_file` | `env`, `envFiles` |
| `depends_on`, `restart` | `dependsOn`, `restart` |
| named `volumes` | `volumes` |
| `deploy.replicas`, `deploy.resources.limits` | `scale.replicas`, `resources` |
| `ports: "8080:80"` | `port: 80` + hint: add `public` if it should be internet-facing |

The converter is loud about what it can't map (bind mounts, CMD healthchecks, custom network
topologies) — a "review these lines" report, never a silent drop. Because railyard's
networking is deliberately compose-identical (service name = hostname, one shared network),
converted apps' internal URLs like `http://db:5432` work unchanged, and users can keep compose
for local dev alongside `.railyard.json` for deployment. If demand appears, a direct
`railyard up -f docker-compose.yml` can later run the converter in-memory — the architecture
doesn't foreclose it.

## Container runtime: Docker or Podman

Nothing in the model requires Docker specifically. The server talks to the runtime through the
**Docker Engine REST API**, which Podman implements via its compatibility socket
(`podman system service` / `podman.socket`), so one client (e.g. bollard) with a configurable
socket path covers both. The primitives railyard needs — user-defined bridge networks with
DNS aliases (Netavark + Aardvark-DNS in Podman ≥4), Dockerfile builds (buildah), named
volumes, events — exist in both.

Two consequences of daemonless worth designing for:

- **Supervision is railyard's job.** Without dockerd, `restart: always` has no daemon to
  enforce it. The railyard server *is* a long-running daemon, so it owns restart policies,
  healthchecking, and cron for both runtimes — the runtime is reduced to "start/stop/build,"
  which keeps behavior identical across Docker and Podman.
- **Rootless caveat:** under rootless Podman the host can't route to container IPs directly,
  so the (host-process) proxy can't reach `api:8080` on the bridge. Either run the runtime
  rootful (proxy reaches container IPs as on Docker), or publish public services to
  localhost-only high ports for the proxy to target. Rootful is the documented default;
  internal service-to-service DNS works in both modes.
