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
source archive, so those values are deployment-pinned by nature and *do* roll back with a
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
  release_id       TEXT NOT NULL,
  activated_by     TEXT,               -- user id, or 'up' | 'rollback' | 'auto-rollback'
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
  name             TEXT NOT NULL,      -- UNIQUE(project_id, name)
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

Migration from the current `deployments` table: it holds upload receipts (id, status,
message, timestamps) — those map onto `releases` with `seq` backfilled by `created_at`
order and `manifest_json` backfilled from the archived source trees where available.

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
