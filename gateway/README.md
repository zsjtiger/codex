# gateway

Rust + axum backend for the cloud Codex web app. It sits between the browser and
per-session `codex app-server` processes (see `../codex-rs/app-server`).

## Status

Skeleton only:

- `GET /healthz` returns `ok`.
- `GET /ws` upgrades to a WebSocket and echoes text frames.

Not yet implemented: authentication, session runtime (starting `codex app-server`),
JSON-RPC forwarding, persistence.

## Run

```sh
cd gateway
cargo run                       # listens on 127.0.0.1:8080
GATEWAY_ADDR=0.0.0.0:9000 cargo run
RUST_LOG=debug cargo run
```

## Layout (planned)

```
src/
├── main.rs      # startup and routing
├── api/         # REST: auth, threads, attachments
├── ws/          # browser WebSocket and JSON-RPC forwarding
├── runtime/     # SessionRuntime trait and implementations
├── auth.rs
└── store/       # Postgres access
```
