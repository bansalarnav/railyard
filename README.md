# Railyard

## Docker Dev Server

Run the server inside the Ubuntu dev container with hot reload:

```bash
./dev
```

The container only publishes the server port on `3000`. For local dev, the primary dashboard URL is `http://127.0.0.1.nip.io:3000`.

Container-side Cargo state is stored in the gitignored `.server/` directory at the repo root so rebuilds stay warm without polluting the main workspace.

## Host-Based Dev Routing

The server now routes by the incoming `Host` header, which matches the long-term reverse-proxy shape:

- `http://127.0.0.1.nip.io:3000` is the dashboard and returns `Hello World`
- `http://howdy.127.0.0.1.nip.io:3000` is a deployment-style subdomain and returns `Howdy World`
- `http://localhost:3000` still works as a convenience alias for the dashboard
- `http://howdy.localhost:3000` still works when your browser resolves `*.localhost`

The base domain is controlled by `BASE_DOMAIN` and defaults to `127.0.0.1.nip.io` in Docker dev. That keeps local routing aligned with the eventual production model of:

- dashboard on the base domain
- deployed apps on `*.<base-domain>`
