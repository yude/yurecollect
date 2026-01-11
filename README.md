# yurecollect

A small Rust program that connects to a WebSocket endpoint, reads incoming JSON (arrays or objects), prints each received message to stdout, and keeps an in-memory buffer capped at roughly 1GB.

## Build

```bash
cargo build --release
```

## Run

Provide the WebSocket URL via CLI arg or `WS_URL` env var:

```bash
# Using CLI arg
cargo run --release -- ws://localhost:8080/ws

# Using env var
WS_URL=ws://localhost:8080/ws cargo run --release
```

- Prints each text frame to stdout.
- Stores raw messages in an in-memory buffer up to ~1GB, evicting oldest when full.
- Attempts to parse JSON (`serde_json::Value`); parse errors are logged to stderr but do not stop the program.
