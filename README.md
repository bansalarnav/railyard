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
railyard-server serve
```

## Docker Dev Server

Run the server inside the Ubuntu dev container with hot reload:

```bash
./dev
```

The container only publishes the server port on `3000`. For local dev, the dashboard URL is `http://127.0.0.1:3000`.

Container-side Cargo state is stored in the gitignored `.server/` directory at the repo root so rebuilds stay warm without polluting the main workspace.

## Dev Routing

The dev proxy listens on `0.0.0.0:3000` inside Docker and forwards requests to the internal API on `127.0.0.1:3001`.

Use `PROXY_HOST`, `PROXY_PORT`, `API_HOST`, and `API_PORT` to change those bind addresses when needed.
