# CLI

The client CLI (`railyard`) is the only interface most users touch. Design rules:

- **The manifest is the source of truth.** There are no `railyard config set …` style commands
  that mutate service settings on the server — you edit `.railyard.json` and run `railyard up`.
  The only server-side state managed imperatively is what deliberately *cannot* live in the
  file: secrets, users, and the GitHub wiring.
- **Verbs at the top level, nouns as groups.** The daily loop (`init`, `up`, `logs`, `status`)
  is one word, like Railway. Management surfaces (`user`, `secrets`, `profile`, `server`) are
  noun groups with `add`/`remove`/`list` subcommands.
- **Every command that talks to a server resolves a profile** (see below). Identity is the
  profile; the project is the manifest. The two are matched automatically and can always be
  forced with `--profile`.
- `--json` on read commands for scripting; human tables otherwise.

## Command tree

```
# Getting connected
railyard server setup <user@host>    # install railyard-server over SSH, start it, log in as admin
railyard login <blob | user@host>    # redeem an invite blob (or mint+redeem over SSH)
railyard logout [--profile <name>]
railyard whoami

# Daily loop
railyard init                        # create project on server, scaffold .railyard.json
railyard link [<project>]            # adopt an existing server project into this directory
railyard unlink
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
railyard user add <name> [--project] # create user, print invite blob
railyard user remove <name>
railyard user list
railyard github link [owner/repo]    # webhook + deploy key + write github block

# Multi-server
railyard profile list
railyard profile rename <old> <new>
railyard profile remove <name>

# Misc
railyard metrics [<service>]         # cpu / memory / restarts per service
railyard completion <shell>
```

## Profiles: the multi-VPS story

A **profile** is one identity on one server: `{server_url, key_id, private_key_path,
project_id?}` at `~/.config/railyard/client/profiles/<name>.json`, written by `login`. Per
[auth](auth.md), a person with three projects on two VPSes simply has three profiles; an admin
of a VPS has one admin profile for that whole server.

Default profile names come from the invite payload: the project name for project-scoped
invites, the server hostname for admin invites — so `railyard login <blob>` needs no flags
and `--profile` is only for overriding collisions.

### Resolution

Project commands (`up`, `logs`, `status`, …) pick a profile in this order:

1. `--profile <name>` flag, then `RAILYARD_PROFILE` env var.
2. The recorded binding for this project: global `config.json` keeps a `projects` map of
   `prj_… → profile name`, written by `init`, `link`, or the first successful match.
3. Scan all profiles against `project.id` from the manifest: a profile scoped to exactly that
   project wins; otherwise admin profiles whose server knows the project are candidates. One
   candidate → use it and record the binding. Multiple → prompt (error with the list when not
   a TTY).

Commands with no project context (`user add` from outside a repo, `whoami`) use steps 1–2,
then fall back to "the only profile" if exactly one exists, else prompt. Nothing about
profiles is ever written into `.railyard.json` — the file is committed and shared; identity
is per-machine.

## Getting connected

Three entry points, one mechanism (the invite blob from [auth](auth.md)):

- **`railyard server setup <user@host>`** — day-zero bootstrap. SSHes to the box, installs the
  `railyard-server` binary (detect arch, download release, systemd unit), runs
  `railyard-server up`, then mints an admin invite and redeems it locally. One command from
  fresh VPS to logged-in admin. Flags: `--version`, `--no-install` (server already present,
  just log in).
- **`railyard login <blob>`** — the normal path for everyone who isn't the machine admin:
  paste the blob a teammate sent you. Generates the keypair, redeems, writes the profile.
- **`railyard login <user@host>`** — sugar for admins with SSH access: runs
  `railyard-server user add` remotely and redeems the result in one step, no blob copying.
  The argument is disambiguated by the `ryd-invite-v1.` prefix.

`logout` deletes the profile and its private key locally and (best-effort) asks the server to
revoke the key. `whoami` prints the active profile, server, user name, and scope — the first
thing to run when a command hits a 403.

## init, link, up

`railyard init` creates the project on the server (`POST /projects`), scaffolds
`.railyard.json` (compose conversion or Dockerfile scan per [manifest](manifest.md)), writes
`project.id`, and records the profile binding. If the file already has a `project.id`, `init`
refuses and points at `link`.

`railyard link` is the inverse for cloning an already-deployed repo on a new machine, or
adopting a server project into an existing file: pick the project (arg or interactive list),
write `project.id` if missing, record the binding.

`railyard up` is declarative sync, and it does **not** invent projects:

- Manifest has no `project.id` → prompt "create project `<name>` on `<server>`?" (a TTY-only
  shortcut for `init`; plain error otherwise). This covers the "manifest exists but project
  doesn't" case explicitly rather than silently.
- `project.id` present but unknown to the resolved server → hard error naming the server, and
  a hint to check `--profile` or run `link`. Auto-creating here would silently fork a project
  onto the wrong VPS.
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

- `railyard user add <name>` — run by an admin: creates an **admin** user unless `--project`
  is given. Run inside a project directory by a project-scoped profile: creates a user scoped
  to *that* project (the server enforces that scoped users can only ever mint their own
  scope, per [auth](auth.md) — this subsumes the `project add-user` command sketched there).
  Prints the invite blob.
- `railyard user remove <name>` / `railyard user list` — scoped to what the profile can see:
  admins see everyone on the server, project users see their project's users.

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
