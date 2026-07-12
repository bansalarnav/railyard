# Auth

Railyard has no passwords and no bearer tokens. A **user** is an identity on the server, a
**key** is an ed25519 keypair on a device, and every API request is signed with a key. The
private key never leaves the client machine; the server stores public keys only, so there is
no credential on the server worth stealing. Enrollment happens through single-use **invite
blobs** that a client redeems by registering its own keypair.

## Users

Exactly two kinds of user, distinguished by one field:

| Kind | `project_id` | Can do |
| --- | --- | --- |
| Admin | absent | Everything on the VPS: all projects, server config, user management. |
| Project-scoped | `prj_…` | Everything within that one project, nothing outside it. |

There is deliberately no membership table and no roles. If the same person needs access to
multiple projects, they get one user per project — on the client that is just one profile per
project (`railyard up --profile acme`), each with its own keypair.

A user can have multiple keys (laptop, desktop, CI). Removing a user revokes all of its keys.
Deleting a project removes its scoped users.

## Invite blobs

Creating a user prints an invite blob:

```
ryd-invite-v1.<base64url JSON>
```

The JSON payload is self-describing: `server_url`, `server_name` (the server's human name —
`RAILYARD_SERVER_NAME` env or the box's hostname — since `server_url` is often a bare IP),
`invite_token`, `expires_at`, and for project-scoped invites the project id and name. The
client derives the profile name from these: project name, else server name, else the URL
host. Properties:

- **Single-use** — redeeming it consumes it; a leaked already-redeemed blob is worthless.
- **Short-lived** — expires after 24 hours if unredeemed.
- **Not a credential** — the server stores only a hash of the token, and the token itself
  never authenticates API requests. It is exchanged, once, for a key registration.

Redemption is the one unauthenticated endpoint: the client generates a keypair locally and
calls `POST /auth/redeem-invite` with the token and its public key. The server verifies the
token, binds the key to the invited user, marks the invite used, and returns the `key_id`.

## CLI lifecycle

On the server (requires SSH to the box — only an admin of the machine mints admins).
User commands talk to the running daemon over a local admin socket (0600, so only the box's
user reaches it): both CLIs manage users through the same API, and only the daemon's
process ever opens the database.

```
railyard-server user add alice              # create admin user, print invite blob
railyard-server user add bob --project prj_…# create project-scoped user, print blob
railyard-server user list
railyard-server user remove bob             # delete user + revoke all their keys
railyard-server auth list-keys
railyard-server auth revoke-key <key_id>    # revoke one device, keep the user
```

On the client:

```
railyard login <blob>                       # generate keypair, redeem invite, write server entry
railyard user add bob                       # create a user scoped to the current project
                                            #   (from .railyard.json project.id), print blob
railyard user add bob --server hetzner      # create a server-wide admin on that server, print blob
railyard login <ssh_target>                 # bootstrap sugar: runs `user add` over SSH and
                                            #   redeems the blob in one step
```

Creating an invite of any kind — project-scoped or server-wide — requires an **admin**;
the server rejects invite requests signed by a project-scoped key.

## Request signing

Every protected request carries these headers, and the server middleware rejects the request
unless all checks pass:

| Header | Contents |
| --- | --- |
| `x-railyard-signature-version` | `v1` |
| `x-railyard-key-id` | Which key signed the request. |
| `x-railyard-timestamp` | Unix seconds; rejected outside a ±300s window. |
| `x-railyard-nonce` | Random per-request value; replays are rejected. |
| `x-railyard-content-sha256` | Hex SHA-256 of the request body. |
| `x-railyard-signature` | ed25519 signature over the canonical request. |

The canonical request (defined in `packages/auth`, shared by client and server) covers the
key id, timestamp, nonce, method, path + query, host, and body hash, so none of those can be
tampered with in transit.

## Authorization

After signature verification, the middleware resolves key → user and attaches it to the
request; handlers apply one rule:

- Admin user → request allowed.
- Project-scoped user → the request must target that user's project; anything else is 403.
  Routes under `/api/projects/{project_id}` sit behind a shared project-access guard
  (scope is checked before existence, so a foreign key cannot probe which ids exist).
  Project listing returns only the caller's project; project creation, the box-wide
  service/routing table, and all user management are admin-only.

## Revocation

| Situation | Action |
| --- | --- |
| Lost or compromised device | `railyard-server auth revoke-key <key_id>` — user keeps other devices. |
| Person leaves | `railyard-server user remove <name>` — user and all keys gone. |
| Project wound down | Deleting the project removes its scoped users. |
