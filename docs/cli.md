# CLI

The client CLI (`railyard`) is the only interface most users touch. Design rules:

- **The manifest is the source of truth.** There are no `railyard config set …` style commands
  that mutate service settings on the server — you edit `.railyard.json` and run `railyard up`.
  The only server-side state managed imperatively is what deliberately *cannot* live in the
  file: secrets, users, and the GitHub wiring.
- **Verbs at the top level, nouns as groups.** The daily loop (`init`, `up`, `logs`, `status`)
  is one word, like Railway. Management surfaces (`user`, `secrets`, `server`) are
  noun groups with `add`/`remove`/`list` subcommands.
- **Every command that talks to a server resolves a server entry** (see below). Identity is
  the server entry; the project is the manifest. The two are matched automatically and can
  always be forced with `--server`.
- `--json` on read commands for scripting; human tables otherwise.

## Command tree

```
# Getting connected
railyard server setup <user@host> [--name <name>] # install server, start it, log in as admin
railyard login <blob | user@host> [--name <name>] [--user <name>] # redeem an invite (or mint+redeem over SSH)
railyard logout [--server <name>]
railyard whoami

# Daily loop
railyard init                        # create project on server, scaffold .railyard.json
railyard unlink                      # forget this project's server binding (manifest untouched)
railyard up [<service>…]             # validate, upload, diff, sync   (--env, --prune, --dry-run)
railyard status                      # per-service state, replicas, domains, last deploy
railyard logs [<service>]            # runtime logs                   (-f, -n, --build, --deploy <id>)
railyard restart <service>
railyard rollback [<service>]
railyard open [<service>]            # open the public URL in a browser

# Server-side state that can't live in the manifest
railyard secrets set KEY=VALUE …     # (--env staging)
railyard secrets unset KEY
railyard secrets list                # names + updated-at, never values
railyard user add <name> [--server <name>] # invite to the current project; --server pins the server
railyard user add <name> --admin [--server <name>] # invite a server-wide admin
railyard user remove <name>
railyard user list
railyard github link [owner/repo]    # webhook + deploy key + write github block

# Multi-server
railyard server list
railyard server rename <old> <new>
railyard server remove <name>

# Misc
railyard metrics [<service>]         # cpu / memory / restarts per service
railyard completion <shell>
```

## Servers: the multi-VPS story

A **server entry** is the local connection and identity for one server:
`{server_url, key_id, private_key_path}` at
`~/.config/railyard/client/servers/<name>.json`, written by `login`. A person deploying to two
VPSes has two server entries; project-to-server bindings are stored separately in the global
client config.

Server names are derived from the invite payload, so the common path needs no flag. Since
`server_url` is realistically a bare IP, the server carries a human name and embeds it in
every invite it mints: set with `railyard server setup --name hetzner` (or later via server
config), defaulting to the box's OS hostname. If the invite has no usable name, `login` falls
back to the URL host. `railyard login --name <name>` overrides the local name explicitly.

There is no magic `default` server — a name like `default` carries no information the day a
second server shows up, and the "I only have one server" case is already handled by
resolution, not by a special name. `railyard login <blob>` therefore needs no flags, first
time or fifth.

When the derived name is already taken:

- **Same server, same user** — this is a re-login (new device key for the same identity).
  Update the server entry in place with the new `key_id`.
- **A different server with the same derived name** — refuse to overwrite the existing entry;
  pass `--name <name>` to choose another local name.

Redeeming an **admin** invite for a server where project-scoped entries exist removes those
entries (and their key files) and repoints their project bindings at the new admin entry —
an admin identity covers every project on the server, so the narrower entries are redundant.
The only way to hold both was joining a project first and being promoted to admin later.

### Resolution

Project commands (`up`, `logs`, `status`, …) pick a server in this order:

1. `--server <name>` flag.
2. The recorded binding for this project: global `config.json` keeps a `projects` map of
   `prj_… → server name`, written by `init`, and by `login` when redeeming a project-scoped
   invite — the blob names the project, so an invited teammate is bound the moment they log
   in.
3. No binding → the command quietly checks every server this machine could act on (admin
   identities, or one scoped to that very project) for the manifest's `project.id`, and
   offers to link the match: one match → y/n prompt, several → a picker (non-interactive
   runs get an error naming what was found). Only when no server has the project does it
   error, pointing at `railyard init` — unless some servers couldn't be checked
   (unreachable, rejected key), in which case the error names them instead of suggesting
   `init`, which would recreate the project. There is deliberately no `link` command —
   linking happens where it is needed.

The manifest is found in the current directory or the nearest ancestor; acting on an
ancestor's manifest asks for confirmation first (non-interactive runs error, naming where
the manifest is). A binding whose server entry has since been removed is treated as no
binding: project commands note the stale link and fall back to discovery. `railyard unlink`
drops the binding explicitly — the manifest keeps its `project.id` — which is how a project
moves servers: `unlink`, then `init` against the new one.

Only commands where a server gets **chosen** prompt: `init` shows an interactive picker with
several servers and no `--server` (error with the list when not a TTY), and `user add
--admin` does the same over the entries holding admin identities. Everything
else, including commands with no project context (`whoami`), accepts
`--server` or falls back to "the only server" if exactly one exists, else errors. Nothing
about server selection is ever written into `.railyard.json` — the file is committed and
shared; identity is per-machine.

## Getting connected

Three entry points, one mechanism (the invite blob from [auth](auth.md)):

- **`railyard server setup <user@host>`** — day-zero bootstrap. SSHes to the box, installs the
  `railyard-server` binary (detect arch, download release, systemd unit), runs
  `railyard-server up`, then mints an admin invite and redeems it locally. One command from
  fresh VPS to logged-in admin. Flags: `--name hetzner` (the server's human name, embedded in
  every invite it mints; defaults to the box's hostname), `--version`, `--no-install` (server
  already present, just log in).
- **`railyard login <blob>`** — the normal path for everyone who isn't the machine admin:
  paste the blob a teammate sent you. Generates the keypair, redeems, writes the server entry.
- **`railyard login <user@host>`** — sugar for admins with SSH access: runs
  `railyard-server user add` remotely and redeems the result in one step, no blob copying.
  The argument is disambiguated by the `ryd-invite-v1.` prefix.

`logout` deletes the server entry and its private key locally and (best-effort) asks the
server to revoke the key. `whoami` prints one row per server entry — user name and scope,
queried live from each server so revoked keys and unreachable boxes show up honestly — and
stars the entry commands in the current directory would use (computed with the same
resolution rules those commands apply). It is the first thing to run when a command hits a
403. `--server <name>` narrows it to one entry.

## init and up

`railyard init` creates the project on the server (`POST /projects`), scaffolds
`.railyard.json` (compose conversion or Dockerfile scan per [manifest](manifest.md)), writes
`project.id`, and records the server binding. When the manifest's ID already exists on the
selected server, it offers to link this directory to that project or create a distinct project
with a new ID.

`init` is idempotent: when the manifest's project already has a recorded binding it prints
the existing link and exits 0, pointing at `railyard unlink` for moving to another server
(`--server` naming a different server errors with the same hint). Run in a subdirectory of
an existing project (a manifest in an ancestor directory), `init` asks for confirmation
before scaffolding a separate nested project; non-interactive runs error.

`init` is also where a server gets **chosen**. `--server <name>` wins. With exactly one
known server, use it and print the target
(`Creating project acme on hetzner (https://…)`), so a single-VPS user never thinks about
this. With several servers, show an interactive terminal picker listing server name + URL;
non-interactive runs must pass `--server`. The choice is then durable: the `project.id`
written to the manifest plus the recorded binding pin every subsequent command (`up`, `logs`,
`user add`, …) to that server, so two projects on two VPSes coexist with no per-command flags.

A manifest may already carry a `project.id` — a cloned repo somebody else deployed, or a
project being brought to a second server. After choosing a server, `init` checks that
server's projects:

- The selected server already has the ID → interactively offer to link this directory to the
  existing project, or create a new project and replace the manifest's ID. Non-interactive runs
  stop and ask the user to rerun `init` interactively.
- The selected server does not have the ID → create the project there under the **same**
  ID. One project keeps one identity across servers, so deploying to a second VPS never
  orphans the manifest's pointer to the first.

`railyard up` is declarative sync, and it does **not** invent projects:

- Manifest has no `project.id` → prompt "create project `<name>` on `<server>`?" (a TTY-only
  shortcut for `init`; plain error otherwise). This covers the "manifest exists but project
  doesn't" case explicitly rather than silently.
- `project.id` present but unknown to the resolved server → hard error naming the server, and
  a hint to check `--server` or run `init`. Auto-creating during `up` would silently fork a
  project onto the wrong VPS.
- Existing project → upload changed path-service snapshots, print the diff plan (create /
  update / no-op per service), stream the rollout. Removal of services requires `--prune`,
  `--dry-run` prints the plan and stops, positional `railyard up api worker` restricts the
  sync to named services.

`up` streams build + rollout progress until the deploy is healthy or rolled back; `--detach`
returns after upload. Exit code reflects rollout success so CI can gate on it.

## Observability

- `railyard status` — one row per service: state (running/deploying/crashed/rolled-back),
  replicas ready/desired, image or commit, public domains, last deploy age. `--env staging`
  scopes to an environment. This is the feedback loop for GitHub-triggered deploys too.
- `railyard logs [service]` — no service argument means all services interleaved with a name
  prefix, compose-style. `-f` follows, `-n 200` tails, `--build` shows the most recent build
  log instead of runtime output, `--deploy <id>` targets a specific deployment.
- `railyard metrics [service]` — point-in-time cpu / memory / restart counts per replica from
  the runtime stats API. Deliberately a table, not a TUI; dashboards can come later.

## Users

Client-side user management is the thin authenticated wrapper over the server's user store —
same data as `railyard-server user …` but over the API, so admins don't need SSH for routine
invites:

- `railyard user add <name>` — creates a user scoped to the **current project** (from
  `.railyard.json` `project.id`) and prints the invite blob; `--server <name>` pins which
  server entry to use, like every other project command. `--admin` creates a **server-wide
  admin** instead — with several servers, a picker over the entries holding admin identities.
  A directory with no linked project offers the admin invite interactively on a TTY (default
  no) and errors otherwise. Either way the server only honors the request from an admin key —
  project-scoped users cannot mint invites (see [auth](auth.md)).
- `railyard user remove <name>` / `railyard user list` — admin-only, like all user
  management; a project-scoped key gets a 403. (Letting project users see their own
  project's members can come later.)

Key revocation (lost laptop) stays server-side (`railyard-server auth revoke-key`) for v1;
an authenticated `railyard user keys` / `key revoke` can be added later without new concepts.

## Deliberately not commands

- **Per-setting mutation** (`railyard domain add`, `railyard scale`, `railyard env set FOO=…`
  for non-secrets) — that's the manifest's job; imperative twins would create drift.
- **`railyard down`** — deleting a project is rare and destructive; keep it on the server CLI
  until there's a real need.
- **`railyard run` / local dev orchestration** — docker-compose already does this; the
  converter keeps compose usable locally.
- **`environment` as a stateful selection** (Railway persists a chosen environment) — an
  invisible mode is a footgun; `--env` is explicit per invocation, default `production`.
