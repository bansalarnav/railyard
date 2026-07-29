# Releases, builds, and containers

How railyard models "what should be running" and "what is actually running." The short
version: an immutable, git-like DAG of desired state (releases → service releases → builds),
one mutable ref per project (the active release), an append-only reflog, and a reconciler
that makes containers match the ref.

Vocabulary is deliberate and small. The word **"deployment" is banned from the model** — it
means three different things depending on who is talking. The four nouns:

| Noun               | One per…                          | Mutable?     | Created…                              |
| ------------------ | --------------------------------- | ------------ | ------------------------------------- |
| **Release**        | `up` / secret rotation            | never¹       | every release                         |
| **ServiceRelease** | service × release                 | never        | every release, one row per service    |
| **Build**          | unique (source, build config)     | status only  | only when the content hash is new     |
| **Container**      | actual container lifecycle        | status only  | only when build or resolved config changed |

¹ pipeline status (`unpacking → ready → building → deployed | failed`) progresses once,
then the row is frozen. Whether a release is *live* is never a row status — see refs below.

Two of these are **records** (releases, service releases): cheap, append-only paper trail,
minted on every release. Two are **resources** (builds, containers): expensive, shared,
reused across releases via pointers and content identity.

## The static shape

```
Project
 └── Release            one per `up` / secret rotation
     │                  immutable: manifest snapshot, message, seq (#42)
     │
     └── ServiceRelease one row per service, ALWAYS created fresh
         │              thin pointer row: action + spec + build pointer
         │
         └──→ Build     content-addressed image; often points at an OLD one

Containers are NOT pointed at by service releases. They are actual state,
connected to specs by content identity + the active ref (see reconciler).
```

## The same thing over time

`●` = created at this release; everything else is reused from an earlier release.

```
              Release #41         Release #42         Release #43
              "checkout page"     "fix api bug"       "bump WORKERS=4"
              ───────────────     ───────────────     ────────────────
web           ● SR (rebuild)      ● SR (unchanged)    ● SR (unchanged)
   build      ● B7                B7                  B7
   container  ● C12               C12                 C12

api           ● SR (unchanged)    ● SR (rebuild)      ● SR (config_only)
   build      B5                  ● B8                B8
   container  C9                  ● C13               ● C14 (restart, same image)
```

`web` was built and started once at #41 and nobody has touched the container since — but
every release still has a row for it, so "show me web's state as of #43" resolves without
walking history.

**Invariant: every release fully describes running state.** Even `skipped`/`unchanged`
services get a row pinning their build and spec. One release's rows are the whole answer;
history is never replayed to reconstruct state.

## Git-like identity

IDs are hashes, split into two kinds:

- **Content-addressed** — the ID *is* the content hash, so dedupe is true by construction:
  - `bld_<sha256(project_id, source_hash, build_hash)>` — builds are like git blobs.
    Computing the ID answers "does this build exist." `service_name` is deliberately NOT
    in the hash: two services sharing a path and dockerfile but differing in `start`
    (e.g. `api` and `worker` in the manifest docs example) hash to the same build and the
    image builds once. `project_id` IS in the hash so images never dedupe across projects.
  - `rel_<sha256(previous, manifest_json, secrets_json, message, created_by, created_at)>`
    — releases are like git commits. Folding `previous` into the hash makes the chain
    self-verifying; a release's identity captures its entire history. Two identical `up`s
    still get different hashes (different parent/timestamp) — correct, releases are
    *events*, builds are *content*.
- **Event IDs** — random hex for things with no meaningful content identity:
  `srl_<hex>`, `ctr_<hex>`.

Mutable status never goes into hashed content — status is like a git ref, living beside
the object; the object itself is immutable.

Display like git: store full hashes, show 7–8 chars, accept unambiguous prefixes anywhere
an ID is accepted (`railyard logs --release f3a92c1`). `seq` (#42) coexists with the hash:
hash for identity, seq for humans.

The hash inventory is one per layer, each meaning exactly one thing:

| Hash                            | Identity of…                                    |
| ------------------------------- | ----------------------------------------------- |
| `builds.id`                     | the build artifact (content)                    |
| `service_releases.config_hash`  | the symbolic spec: `hash(config_json)`          |
| `containers.resolved_hash`      | the concrete running config (values resolved)   |

## Rollback = moving a ref, not making a commit

There is no rollback release and no `rollback_of` column. **Releases are commits; "active"
is a ref.** `railyard rollback` is `git checkout`: the project's `active_release` pointer
moves to an old release and the reconciler makes reality match. An append-only
`activations` table — literally a reflog — records every ref move, which is what keeps
"what was running at 3am" answerable.

Branching falls out naturally. After rolling back to #41, the next `up` sets
`previous = #41` (the ref target, not #42). The bad release becomes an abandoned tip:

```
#40 ── #41 ── #42        ← bad; abandoned tip, kept forever in history
         └─── #43        ← next `up` after rollback to #41
```

**Branching never implies merging.** The manifest file in the repo is always the source of
truth for the next release's content, so branches are historical record, not divergent
lines needing reconciliation. `seq` keeps incrementing globally — it is a counter, not a
chain position.

Since any past release is re-activatable, image GC must respect that: prune builds
unreachable from the last N releases (retention policy), not "not currently active."
Rolling back to something pruned re-builds from the archived source, which is kept.

### The local manifest file is never touched

Config in the file always wins over server-side state (see `cli.md`). Rollback is a
temporary, server-side divergence from the repo; the next `up` re-applies the file —
possibly redeploying the thing that was rolled back, which is correct declarative
behavior, and the CLI should loudly say so. `rollback` prints the diff between the file
and the now-active release; a `railyard config pull` writes the active release's manifest
back into the repo when the user decides the rollback is permanent — then *they* commit
it. Silently editing the working tree would fight git.

## Environments are refs

*Designed, not built. Supersedes manifest `environments` overlays, removed in `2ed1a25`.*

A project has one release DAG and **N named refs** over it. `production` always exists;
`railyard up --env staging` creates or moves `staging`; a sandbox for a branch or an agent is
just another ref with an expiry. Each ref has its own container set; the reconciler runs per
`(ref, service)` instead of per service.

```
                #40 ── #41 ── #42 ── #43 ── #44
                        ▲             ▲       ▲
                        │             │       └── staging     (ref)
                        │             └────────── production  (ref)
                        └──────────────────────── agent-42    (ref, leased 24h)
```

**The load-bearing invariant: a release is environment-independent.** Nothing in
`manifest_json` or `config_json` names an environment; `${{ secrets.X }}` stays symbolic and
resolves against the ref's environment at container start. So any release is deployable under
any ref, which is what makes the next paragraph a pointer move rather than a rebuild.

**Promotion is a ref move.** `railyard promote staging production` points `production` at
whatever release `staging` currently holds — no upload, no rebuild, the exact bits that were
tested. Mechanically it is the same operation as `rollback` (both move a ref, both append to
`activations`); only the ref name and the direction of travel differ.

### What legitimately differs per environment

Overlays are *authored* divergence: a second source of truth, kept in sync by hand, that
silently changes what a release means depending on where it lands. Everything below is
**derived by rule** instead, so it needs no manifest keys and cannot drift:

| Divergence          | Mechanism                                                                 |
| ------------------- | ------------------------------------------------------------------------- |
| Secret **values**   | secrets are keyed `(project_id, env, name)`; names are the same everywhere |
| Public domains      | `production` uses the manifest's `public.domains`; other refs get `<env>--<service>.<wildcard base>` from the server's wildcard domain. No wildcard configured → non-production refs are internal-only |
| Scale               | non-production refs clamp to `replicas: 1`, autoscale off                  |
| Volumes / data      | volumes are per ref, created empty (cheap CoW clone of the parent ref is a deferred want — see wrinkles) |

Secret *names* being env-independent is what keeps validation honest: `up --env staging` fails
before building if staging is missing a name the manifest references, listing exactly which.

### Leases and reachability

`production` and other long-lived refs are permanent. Sandbox refs carry `lease_until`;
expiry stops their containers and drops the ref, no dashboard gardening. With refs generalized,
GC generalizes too: a release is **live** if some ref reaches it within the retention policy, a
build/volume/container is live if something live references it, everything else is collectible.
Infra you cannot leak matters much more when most refs are created by agents rather than typed
by hand.

`--env` stays explicit per invocation and is never sticky state (see `cli.md`); `railyard envs`
lists refs with their release, age, and remaining lease.

## Servers are remotes

*Designed, not built.*

`init` deliberately keeps **one project ID across servers**. Combined with
`bld_<sha256(project_id, source_hash, build_hash)>`, that means identical source produces an
identical build ID on every box — so "does the target already have this object?" is answerable
from the ID alone, with no registry and no coordination. That is exactly git's have/want
negotiation, and it makes a second VPS a *remote* rather than an island.

```
hetzner                                        fra-1
  refs/staging    → #44                          refs/production → #42
  builds  B5 B7 B8 B9                            builds  B5 B7 B8

        ──────────── railyard promote #44 --to fra-1 ────────────▶
        want #44 → fra-1 answers "missing: rel #43,#44, bld B9"
        send releases + service_release rows + B9 image + archived source
        fra-1 verifies hashes, moves refs/production → #44
```

The transfer is the delta only, and it is **self-verifying**: the receiver recomputes
`rel_<sha256(previous, manifest_json, secrets_json, message, created_by, created_at)>` down the
chain and every `bld_` from its content, so tampering in flight or a mismatched project ID
fails the push instead of landing.

**Not transferred:** secrets, volumes and their data, containers, logs. The target resolves the
spec with its *own* environment's secrets — which is the point (prod credentials never leave
prod) and the one place a promotion can fail late, so the receiver validates referenced names
up front, exactly as `up` does.

This buys three things without a control plane: a dev/staging box promoting into a prod box; a
warm standby that fetches objects but leaves its ref behind (a ref that is deliberately behind
is the same state `secrets set --stage` already produces); and multi-region as a fan-out of the
same push. Each server stays authoritative for its own refs — promotion is a push, not a global
reconciliation loop, and there is no scheduler that owns the fleet.

Wrinkles: bytes route through the client first (it is already authenticated to both ends, at the
cost of a laptop round-trip for images) — direct server-to-server needs a delegated,
project-scoped, expiring token. Platform mismatch (arm64 source box, amd64 target) must be
checked against the image manifest before transfer and fall back to rebuilding on the target
from the archived source.

## Desired vs actual: the reconciler

Service releases describe **desired state only** (spec + build). Containers are **actual
state**. They connect through content identity, not foreign keys — which is what makes
re-activating an old release sound (its rows never mutate; fresh containers are started
to satisfy old specs).

The reconciler's entire job, per service in the active release:

```
hash(resolve(active SR's config_json))  ==  running container's resolved_hash ?
```

- No match / no container → start one from the spec (build reused via content address).
- Match → nothing to do.
- Containers matching no active spec → stop them (`removed`, or superseded config).

`resolve()` replaces `${{ secrets.X }}` and `${{ services.db.host }}`-style interpolations
with current real values and passes everything else through. `resolved_hash` covers the
**entire** resolved config — port, start command, healthcheck, resources, volumes, restart
policy — not just env; otherwise a healthcheck change would look satisfied by the old
container. If two symbolic specs resolve identically they are behaviorally identical, so
one comparison is the whole check; there is no separate "which spec" match at the
container level (provenance, when needed for diagnostics, is a join through
`created_by_release`).

This single check also catches drift that happens *without* a release: a dependency
crash-restarts on a new address, a secret rotates — fresh resolution differs, affected
containers restart. `railyard status` can run the same comparison read-only and show
"env stale" without restarting anything.

## Change detection at release time

When an archive lands and unpacks, the server hashes each service's subtree (its `path`,
or `build.watch` globs when present) → `source_hash`, plus build-affecting config
(dockerfile, args) → `build_hash`. Then per service, comparing against the currently
active release:

| Comparison result                          | `action`      | Effect                          |
| ------------------------------------------ | ------------- | ------------------------------- |
| new (source_hash, build_hash)              | `rebuild`     | build image, start container    |
| same build, different `config_hash`        | `config_only` | restart with new config, no build |
| same build, same `config_hash`             | `unchanged`   | container untouched             |
| service not selected (`up api worker`)     | `skipped`     | pin current build/spec, untouched |
| service gone from manifest (with `--prune`)| `removed`     | container stopped               |

There is no "diff against previous release" logic — reuse falls out of content
addressing, which also means reverting a commit reuses the old build for free.

`action` describes **spec** changes only. A secret rotation can restart a service whose
latest SR says `unchanged` — correct, because the spec didn't change; the world under it
did. The restart is recorded where actuals live (a new container row), never by
falsifying release history.

`config_hash = hash(config_json)` is purely symbolic (secrets as references), so two
releases with identical service config hash identically regardless of secret state at
their release times.

### `config_json`: the compiled spec

Each service release stores `config_json` — the **normalized spec**: that service's
manifest fragment with defaults applied, structure normalized, `${{ … }}` references left
symbolic. Why store it when the release already has `manifest_json`:

1. **Rollback fidelity across server upgrades.** If manifest resolution logic changes in
   a future server version, an old release still deploys exactly as it originally did —
   the executor reads the frozen compiled form, never re-derives it.
2. `config_hash` is verifiable: it's just `hash(config_json)`.
3. Per-service diffing (`railyard diff`, per-service rollback synthesis) reads two small
   JSON blobs instead of re-parsing manifests.

`manifest_json` on the release remains the source-of-record of what the user wrote;
`config_json` is the compiled artifact.

## The plan

*Designed, not built.*

Every `up` prints a plan before it changes anything; `--dry-run` prints it and stops. Because
source hashing happens server-side after unpack, the plan is computed **server-side**, against a
specific head — it is a real object with an ID, not a client-side guess.

The plan is a **dry-run of the reconciler**, not a parallel implementation of it: it calls the
same `resolve()` and compares the same `resolved_hash`es, with the writes turned off. Anything
else eventually drifts from what apply actually does, which is the failure mode that makes
`terraform plan` output untrustworthy in other tools.

```
Plan for acme → production on hetzner
  ref  production is at #42 (parent of this release) — fast-forward ✓
  git  a1b2c3d on main, clean

  api      rebuild       src 4f21e9a → 9c02b71   image builds
  worker   rebuild       shares api's build       reuses bld_9c02b71
  web      unchanged     —
  db       config_only   memory 1Gi → 2Gi         restart, no build

  collateral
    cache  restart       DATABASE_URL resolves to a new address

  secrets  STRIPE_SECRET_KEY changed since #42 (value hash differs)

  destructive
    mailer removed       container stopped, volume mailer-data retained 7d then GC'd

  3 services change, 1 collateral restart, 1 destructive.
```

Four things the plan carries that a per-service action table alone cannot:

- **The ref line.** Which ref is being moved, where it is now, and whether this release
  fast-forwards it. A non-fast-forward is visible *before* the upload rather than as a rejection
  after it, and it names who moved the ref.
- **Blast radius.** The dependency graph is already implied by `${{ services.<name>.… }}`
  references and `dependsOn`. A service is collateral-restarted exactly when its *resolved*
  config changes even though its spec did not — a dependency's address moved, a secret rotated.
  This is the class of change that surprises people today, because nothing in the release
  history shows it (`action` describes spec changes only).
- **Secrets by name, never by value.** `secrets_json` holds `{name: value_hash}`, so the plan
  can say *which* secret drifted since the current head without reading any value.
- **Destructive operations, called out separately.** Service removal, volume deletion, a domain
  moving between services. The more `up` becomes a full sync of the manifest (no service
  selection, no `--prune` opt-in), the more removals are implied by editing the file rather than
  requested by a flag — which is the right trade, and exactly why they must be impossible to
  miss in the plan.

For non-humans, the plan is the contract:

- `--json` emits a stable shape with `destructive: bool`, so an agent (or CI) can gate on it
  without parsing prose.
- Exit codes follow `terraform plan -detailed-exitcode`: `0` no changes, `2` changes pending,
  `1` error. A no-op `up` is then machine-detectable, which is what makes agent retry loops safe.
- `up --apply <plan_id>` applies **exactly** the plan that was shown, and fails if the ref has
  moved since it was computed. That is the same compare-and-swap the ref already needs, and it
  is what an approval gate ("agent proposes, human applies") is built out of. Plans are
  disposable — recomputing is cheap — so they can live in memory with a short TTL rather than in
  a table.

## Secrets

Project-level, mutable, unversioned KV store (versioning deliberately deferred).
Write-only API: `secrets set` / `secrets rm` / `secrets list` — list returns names and
metadata, never values. Encrypted at rest with a server-held key; the docs should be
honest that this is hygiene, not a boundary — the box that stores them also runs the
containers that receive them in plaintext.

**Rotation creates a release.** `secrets set STRIPE_KEY=…` mints a new release: same
`manifest_json` as the parent, auto-message `"Update secret STRIPE_KEY"`, `previous` =
active. Every change to running state is a release; rotations show up in
`railyard releases` history. Refinements:

- Skip the release when the rotated secret is referenced by no service — just update the KV.
- **Immediate vs staged is just the ref.** Default: create the release and move the ref
  (affected services restart now). `--stage`: create the release, don't move the ref —
  it sits as a forward tip, `status` shows the ref is behind, applying is moving the ref.
  No separate "pending changes" state exists.

**`releases.secrets_json` is a record, not a spec input.** It stores
`{name: value_hash}` for the secrets the manifest references, captured at release time —
per-secret hashes of values, never values. Because the KV is unversioned, an old
release's recorded hashes may be unsatisfiable; resolution therefore **always uses
current values**. Consequences, all intentional:

- Rotation survives rollback: rolling back never un-rotates a credential (the classic
  footgun — rotations are usually security-motivated). Undoing a rotation means
  re-setting the value: an explicit, logged action.
- On `rollback`, the CLI compares the target's `secrets_json` against current value
  hashes and warns by name: *"STRIPE_KEY has changed since #40 deployed; current values
  will be used."*
- `railyard diff #40 #43` can say *which* secret changed without knowing any values.

**envFiles are the other kind of secret, and that's fine.** `.env.api` ships inside the
source archive, so those values are release-pinned by nature and *do* roll back with a
release. The contrast is a feature: env files = config that travels with the code
snapshot; the secrets store = values that outlive releases and rotate independently.
Users pick semantics by picking the mechanism.

Validation: a manifest referencing `${{ secrets.X }}` where `X` doesn't exist fails the
release before anything builds, listing the exact missing names.

## Logs

- **Build logs** attach to the build row. A reused build's logs already exist — another
  win from content addressing.
- **Runtime logs** attach to the container. "Logs for api as of release #42" is a query:
  containers matching #42's spec for `api` whose lifetime overlapped #42's activation
  windows (from the reflog). Unchanged services naturally resolve to a container that
  started under an earlier release.

## Schema

Stays in libsql/SQLite alongside the existing tables.

```sql
projects (
  ...existing columns...,
  active_release   TEXT                -- the ref; NULL before first release
)                                      -- dropped once `refs` lands: it is refs['production']

refs (                                 -- one row per environment; generalizes the single
  project_id       TEXT NOT NULL,      --   projects.active_release column above
  name             TEXT NOT NULL,      -- UNIQUE(project_id, name): production|staging|agent-42
  release_id       TEXT,               -- NULL before this ref's first release
  lease_until      INTEGER,            -- NULL = permanent; expiry reaps ref + containers
  created_at       INTEGER NOT NULL,
  updated_at       INTEGER NOT NULL
)

releases (
  id               TEXT PRIMARY KEY,   -- rel_<hash>, see identity section
  project_id       TEXT NOT NULL,
  seq              INTEGER NOT NULL,   -- UNIQUE(project_id, seq); per-project counter
  previous         TEXT,               -- parent release; NULL only for the first
  manifest_json    TEXT NOT NULL,      -- exact manifest snapshot at release time
  secrets_json     TEXT NOT NULL,      -- {name: value_hash} for referenced secrets
  message          TEXT,
  status           TEXT NOT NULL,      -- pipeline only: unpacking|ready|building|deployed|failed
  error            TEXT,
  created_by       TEXT,
  created_at       INTEGER NOT NULL,
  updated_at       INTEGER NOT NULL
)

activations (                          -- the reflog; append-only
  project_id       TEXT NOT NULL,
  ref_name         TEXT NOT NULL,      -- which ref moved
  release_id       TEXT NOT NULL,
  activated_by     TEXT,               -- user id, or 'up' | 'rollback' | 'promote' | 'auto-rollback'
  activated_at     INTEGER NOT NULL
)

service_releases (                     -- desired state only; immutable
  id               TEXT PRIMARY KEY,   -- srl_<hex>
  release_id       TEXT NOT NULL,
  service_name     TEXT NOT NULL,      -- UNIQUE(release_id, service_name)
  action           TEXT NOT NULL,      -- rebuild|config_only|unchanged|skipped|removed
  config_json      TEXT NOT NULL,      -- normalized spec; secrets left symbolic
  config_hash      TEXT NOT NULL,      -- hash(config_json)
  build_id         TEXT                -- NULL for image:-sourced services
)

builds (
  id               TEXT PRIMARY KEY,   -- bld_<hash(project_id, source_hash, build_hash)>
  project_id       TEXT NOT NULL,
  source_hash      TEXT NOT NULL,      -- hash of the service's file subtree
  build_hash       TEXT NOT NULL,      -- hash of build-affecting config
  image_ref        TEXT,
  status           TEXT NOT NULL,      -- queued|building|succeeded|failed
  log_path         TEXT,
  created_at       INTEGER NOT NULL
)

containers (                           -- actual state
  id               TEXT PRIMARY KEY,   -- ctr_<hex>
  project_id       TEXT NOT NULL,
  ref_name         TEXT NOT NULL,      -- which environment this container belongs to
  service_name     TEXT NOT NULL,
  build_id         TEXT,
  resolved_hash    TEXT NOT NULL,      -- hash of the FULLY resolved config at start
  created_by_release TEXT NOT NULL,    -- provenance only; spec details via join
  status           TEXT NOT NULL,      -- starting|healthy|crashed|stopped
  exit_code        INTEGER,
  log_path         TEXT,
  started_at       INTEGER NOT NULL,
  exited_at        INTEGER
)

secrets (
  project_id       TEXT NOT NULL,
  env              TEXT NOT NULL,      -- UNIQUE(project_id, env, name); names match across envs
  name             TEXT NOT NULL,
  value            BLOB NOT NULL,      -- encrypted at rest
  updated_by       TEXT,
  updated_at       INTEGER NOT NULL
)
```

Notably absent, on purpose:

- **No `services` table.** A service's identity is `(project_id, name)`; it lives in the
  manifest. If services later need server-side state that outlives releases (volume
  metadata, DNS-verified custom domains), that's when a small table earns its existence.
- **No `container_id` on service_releases.** Desired and actual connect through content
  identity + the ref; a foreign key would have to mutate on re-activation.
- **No `config_hash` on containers.** Redundant with `resolved_hash` + provenance join.
- **No per-row deploy status on service_releases.** Deploy-attempt outcomes live with
  containers and activations.
- **No `plans` table.** A plan is derived from (uploaded source, head, current secrets) and is
  cheap to recompute; persisting it would create a second, staler source of truth about
  desired state. Short-lived in memory, keyed by `plan_id`, revalidated against the ref at apply.
- **No `env` on releases or service_releases.** Releases are environment-independent by
  construction — that is what makes promotion a ref move instead of a rebuild.

The current `releases` table is already the upload-receipt subset of this shape:
id, project_id, status, message, error, timestamps. Growing it into
the table above means adding `seq` (backfilled by `created_at` order), `previous`,
`manifest_json` (backfilled from the archived source trees where available),
`secrets_json`, and `created_by`.

## Known wrinkles / deferred decisions

- **Replicas.** At `scale.replicas > 1`, one spec maps to N containers. The reconciler
  check generalizes ("N healthy containers matching resolved_hash") and containers grow a
  `replica_index`; nothing structural changes, but code shouldn't bake in 1:1 too deeply.
- **`config_hash` granularity.** One hash means a routing-only change (adding a domain)
  restarts the container. Splitting runtime-hash vs routing-hash avoids that; additive,
  do later.
- **Where source hashing happens.** Server-side after unpack first. Client-side hashing
  could later skip uploading unchanged subtrees entirely — an optimization, not a schema
  change.
- **`railyard releases --verbose`** should eventually print something like the timeline
  table above.
- **Copy-on-write data for sandbox refs.** A new ref starts with empty volumes today, which
  makes sandboxes cheap but not realistic. On btrfs/ZFS a ref could instead start from a CoW
  snapshot of its parent ref's volumes: near-instant, near-zero disk, reaped with the lease.
  This is what would make per-branch and per-agent environments genuinely useful, and it needs
  no model changes — only volume provisioning that is aware of `refs`.
- **Direct server-to-server transfer.** Promotion routes objects through the client first,
  since it already holds credentials for both ends. Box-to-box needs a delegated,
  project-scoped, expiring token; the object protocol itself does not change.
- **Ref protection.** Which identities may move which refs (an agent key that owns `staging`
  but may only *propose* a `production` move) belongs with auth, not here — but it is the
  reason ref moves are a single, auditable operation in the first place.
