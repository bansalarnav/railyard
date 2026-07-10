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
./dev
```

The container only publishes the server port on `3000`. For local dev, the dashboard URL is `http://127.0.0.1:3000/railyard`.

Container-side Cargo state is stored in the gitignored `.server/` directory at the repo root so rebuilds stay warm without polluting the main workspace.

## Dev Routing

The proxy listens on `0.0.0.0:3000` inside Docker (`127.0.0.1:3000` by default outside it). Requests whose path starts with `/railyard`, or whose hostname starts with the `railyard.` label, are forwarded to the internal API on `127.0.0.1:3001`. Other hostnames are matched against service upstreams configured via `RAILYARD_CONTAINER_UPSTREAM_<NAME>` env vars (e.g. `RAILYARD_CONTAINER_UPSTREAM_WEB=127.0.0.1:4000` routes `web.*` hosts there); unmatched requests get a 404.

Use `RAILYARD_PROXY_HOST`, `RAILYARD_PROXY_PORT`, `RAILYARD_API_HOST`, and `RAILYARD_API_PORT` to change those bind addresses when needed.
