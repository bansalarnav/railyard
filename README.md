# Railyard

## Server Daemon

Run the server in the background:

```bash
railyard-server up
```

Check or control the background server:

```bash
railyard-server status
railyard-server restart
railyard-server down
```

For development or external process supervision, run the server in the foreground:

```bash
railyard-server up --foreground
```

## Docker Dev Server

Run the server inside the Ubuntu dev container with hot reload:

```bash
./dev-server
```

The container only publishes the server port on `3000`. For local dev, the dashboard URL is `http://127.0.0.1:3000/railyard`.

On the first run, the dev server creates an admin user and logs the local CLI in
automatically. Run authenticated client commands from another terminal with:

```bash
./dev-cli whoami
./dev-cli init
```

The gitignored `.dev-state/` directory stores the server database, deployment
data, dev client credentials, and Cargo build state. The server's persistent
data is directly inspectable under `.dev-state/server`; container-only Unix
runtime files such as the admin socket and PID file live under `/run/railyard`.
Restarting or rebuilding the container preserves both projects and login state.
The regular `railyard` CLI continues to use the user's global config directory;
only `./dev-cli` (or `RAILYARD_DEV=1`) uses the repository-local client state.

Outside Docker development, no override is required: server data continues to
default to `$XDG_STATE_HOME/railyard/server` (or
`~/.local/state/railyard/server` when `XDG_STATE_HOME` is unset).

## Dev Routing

The proxy listens on `0.0.0.0:3000` by default. Requests whose path starts with `/railyard`, or whose hostname starts with the `railyard.` label, are forwarded to the internal API on `127.0.0.1:3001`. Other hostnames are matched against service upstreams configured via `RAILYARD_CONTAINER_UPSTREAM_<NAME>` env vars (e.g. `RAILYARD_CONTAINER_UPSTREAM_WEB=127.0.0.1:4000` routes `web.*` hosts there); unmatched requests get a 404.

Use `RAILYARD_PROXY_HOST`, `RAILYARD_PROXY_PORT`, `RAILYARD_API_HOST`, and `RAILYARD_API_PORT` to change those bind addresses when needed.
