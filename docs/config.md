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
| `services` | object | Map of service name → service config. Names are `[a-z0-9-]`, must be unique; the name **is** the internal hostname. |
| `environments` | object | Optional per-environment overrides, deep-merged over the base config (Railway-style). Selected with `railyard up --env staging`; default environment is `production`. |

## Service source (exactly one required)

| Key | Type | Notes |
| --- | --- | --- |
| `path` | string | Directory relative to the config file (same level or deeper — paths outside the repo root are rejected). Built on the server from an uploaded snapshot of that directory. |
| `image` | string | A pullable image reference (`postgres:16`, `ghcr.io/acme/thing:v2`). No build step. |

`path` and `image` are mutually exclusive; a service must have one. Two services may share a
`path` with different `start` commands (e.g. `api` + `worker`).

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
| `healthcheck.timeout` | number | 60 | Seconds to wait for healthy before rollback. |
| `restart` | string | `on-failure` | `always` \| `on-failure` \| `never`. |
| `dependsOn` | string[] | — | Start ordering within a sync; waits for the dependency's healthcheck if it has one. |
| `cron` | string | — | Cron expression. The service becomes a job: run on schedule, exit, not part of the long-running set. |

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
railyard status     # per-service state, replicas, domains
```

`railyard new` flow: verify auth against the server → `POST /projects` → receive `prj_…` id →
if `docker-compose.yml` exists offer to convert it; else scan subdirectories for Dockerfiles and
prefill `path` services → write the file → print `railyard up` as the next step. If the file
already has a `project.id`, `new` refuses and points at `link`.

`up` is declarative: file present on server but identical → no-op; changed → redeploy; new →
create. Removal requires `--prune` so a typo'd rename can't take down a database.
