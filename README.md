# Aethon

## Docker Dev Server

Run the server inside the Ubuntu dev container with hot reload:

```bash
./dev
```

The container only publishes the server port on `http://localhost:3000`.

Container-side Cargo state is stored in the gitignored `.server/` directory at the repo root so rebuilds stay warm without polluting the main workspace.
