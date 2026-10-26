# web

React + TypeScript frontend for the cloud Codex app. Talks to the Rust gateway
in `../gateway` over HTTP and WebSocket.

## Status

Skeleton only: one page that opens `/ws` and sends a sample JSON-RPC `initialize`
frame. Nothing is rendered from Codex events yet.

## Run

```sh
cd gateway && cargo run          # terminal 1: gateway on 127.0.0.1:8080
cd web && npm install            # terminal 2
npm run dev                      # http://localhost:5173, proxies /api, /healthz, /ws
```

Set `GATEWAY_URL` to point the dev proxy elsewhere.

## Layout (planned)

```
src/
├── api/        # REST client and TanStack Query hooks
├── realtime/   # WebSocket connection, JSON-RPC, reconnection
├── store/      # Zustand store and event reducer
├── features/   # threads, approvals, files, terminal
└── components/ # shared UI
```
