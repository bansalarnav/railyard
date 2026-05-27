# Railyard

Railyard is a deployment platform for a VPS you own. The goal is to recreate most of Railway's core workflow without depending on Railway.

It has a server/client architecture:

- The server runs on the user's VPS.
- The client is a local CLI.
- After global initialization, every normal operation should be possible from the local CLI.

Keep code simple, readable, and direct. Do not add tests unless explicitly asked.
