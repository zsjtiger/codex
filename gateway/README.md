# gateway

Rust + axum backend for the cloud Codex web app. It is the control plane between the
browser and per-session `codex app-server` processes (see `../codex-rs/app-server`).

## Status

- REST control plane under `/api/v1`: sessions, workspaces, members, environments,
  threads, turns, attachments, usage and audit log. Backed by Postgres.
- Turns run on codex app-server: local launching per environment, or a configured
  `endpoint`. Turns on a thread run one at a time, oldest first.
- Browser WebSocket at `/ws`: authenticated, streams a thread's app-server events, and
  routes approval requests to the browser.
- Restart recovery: turns interrupted by a restart are failed, queued turns resume.
- Idle local app-servers are stopped after `GATEWAY_IDLE_SECS`.

Not implemented yet: real sign-in (only the dev login exists), object storage for
attachment bytes, rate limiting, and resuming an interrupted turn with `thread/resume`.

## Configuration

| Variable | Default | Meaning |
|---|---|---|
| `DATABASE_URL` | required | Postgres connection string |
| `GATEWAY_ADDR` | `127.0.0.1:8080` | Listen address |
| `GATEWAY_DEV_LOGIN` | unset | `1` or `true` enables `POST /api/v1/auth/sessions`. Dev only |
| `GATEWAY_CODEX_BIN` | unset | Path to the `codex` binary. Environments with no `endpoint` get a local app-server |
| `GATEWAY_CODEX_HOME` | unset | Root for per-environment `CODEX_HOME`s. Required with `GATEWAY_CODEX_BIN` |
| `GATEWAY_IDLE_SECS` | `1800` | Stop a local app-server after this long without a lease |
| `GATEWAY_STORAGE` | `local` | Attachment bytes: `local` (disk) or `s3` |
| `GATEWAY_STORAGE_DIR` | `storage` | Directory for attachment bytes when `GATEWAY_STORAGE=local` |
| `GATEWAY_S3_BUCKET` | unset | Bucket for attachment bytes. Required when `GATEWAY_STORAGE=s3` |
| `GATEWAY_S3_PREFIX` | unset | Key prefix inside the bucket |
| `GATEWAY_S3_ENDPOINT` | unset | Custom endpoint for S3-compatible services such as MinIO |
| `GATEWAY_S3_REGION` | from AWS config | Region for the bucket |
| `GATEWAY_S3_FORCE_PATH_STYLE` | `false` | Set `true` for MinIO and other services that need path-style URLs |

Migrations in `migrations/` run automatically at startup. Each environment's
app-server log is written to `<GATEWAY_CODEX_HOME>/<environment id>/app-server.log`.

## Run and test

```sh
export DATABASE_URL=postgres://postgres@127.0.0.1:5432/codex_gateway
export GATEWAY_DEV_LOGIN=1
cargo run                        # applies migrations, then listens on 127.0.0.1:8080
cargo test                       # integration tests; sqlx creates a fresh database per test
cargo clippy --all-targets

# Also start a real app-server through the gateway (needs a built codex binary):
GATEWAY_TEST_CODEX_BIN=../codex-rs/target/debug/codex cargo test --test realtime -- --ignored
```

The test database role needs `CREATEDB`.

## Data model

Migration `0001_init.sql`:

| Table               | Purpose                                                                 |
|---------------------|-------------------------------------------------------------------------|
| `users`             | Accounts, keyed by lowercase email                                      |
| `sessions`          | Bearer sessions. Stores `sha256(token)` only, with expiry and revocation |
| `workspaces`        | Tenants, addressed by a unique slug                                     |
| `workspace_members` | `owner` / `admin` / `member`. Exactly one owner per workspace           |
| `environments`      | Runtimes that execute threads. `endpoint` is the app-server `ws://` URL |
| `threads`           | Conversations. `codex_thread_id` links to the app-server thread         |
| `turns`             | Ordered inputs per thread (`seq`), with status and error                |
| `attachments`       | File metadata. Bytes live in object storage under `storage_key`         |
| `usage_records`     | Token usage written by the runtime when a turn finishes                 |
| `audit_logs`        | Append-only record of changes, written in the same transaction          |

## Authorization

- Every workspace request needs a bearer token. Non-members get `404`, so workspace
  existence is not revealed.
- Workspace roles: `member` < `admin` < `owner`.
  - Members: read workspace data, create threads and turns, upload attachments, and
    modify their own threads and attachments.
  - Admins: manage members (except owners and other admins), environments, usage and
    audit logs, and modify any thread or attachment.
  - Owner: change roles, delete the workspace.
- The owner role cannot be assigned or removed through the API.

## API

All responses are JSON. Errors use `{"error": {"code", "message"}}`.

| Method | Path | Notes |
|---|---|---|
| POST | `/api/v1/auth/sessions` | Dev login: `{email, display_name?}` returns `{token, expires_at, user}` |
| DELETE | `/api/v1/auth/sessions` | Revoke the current session |
| GET | `/api/v1/me` | Current user |
| GET, POST | `/api/v1/workspaces` | List mine / create (caller becomes owner) |
| GET, PATCH, DELETE | `/api/v1/workspaces/{id}` | PATCH: admin. DELETE: owner |
| GET, POST | `/api/v1/workspaces/{id}/members` | POST `{email, role?}`: admin. Email must already have an account |
| PATCH, DELETE | `/api/v1/workspaces/{id}/members/{user_id}` | PATCH role: owner. DELETE: admin, or self to leave |
| GET, POST | `/api/v1/workspaces/{id}/environments` | POST: admin |
| GET, PATCH, DELETE | `/api/v1/environments/{id}` | PATCH: admin (status, endpoint, name) |
| GET, POST | `/api/v1/workspaces/{id}/threads` | List: `?archived=&limit=&offset=`. Create: `{title?, environment_id?}` |
| GET, PATCH, DELETE | `/api/v1/threads/{id}` | Modify: creator or admin. PATCH `{title?, archived?}` |
| GET, POST | `/api/v1/threads/{id}/turns` | Create returns `201` with status `queued`. Rejected on archived threads |
| GET, POST | `/api/v1/workspaces/{id}/attachments` | Metadata only. `?thread_id=` filters the list |
| DELETE | `/api/v1/attachments/{id}` | Uploader or admin |
| GET | `/api/v1/workspaces/{id}/usage` | Admin. `?from=&to=` (RFC 3339), defaults to the last 30 days |
| GET | `/api/v1/workspaces/{id}/audit-logs` | Admin. `?limit=&offset=` |
| GET, POST | `/api/v1/workspaces/{id}/projects` | POST `{name, instructions?}`. Instructions go into every new thread in the project |
| GET, PATCH, DELETE | `/api/v1/projects/{id}` | Edit or delete: creator or admin |
| GET, POST | `/api/v1/workspaces/{id}/skills` | Members read. Create: admin. Enabled skills go into new threads |
| GET, PATCH, DELETE | `/api/v1/skills/{id}` | PATCH `{enabled?, name?, description?, instructions?}`: admin |
| GET, POST | `/api/v1/workspaces/{id}/artifacts` | List filters: `?project_id=&kind=`. Kinds: document, slides, design, dashboard |
| GET, PATCH, DELETE | `/api/v1/artifacts/{id}` | Edit or delete: creator or admin. `project_id: null` removes the project |
| GET | `/api/v1/workspaces/{id}/search?q=` | Case-insensitive search of chats, message text, artifacts and projects |
| GET, POST | `/api/v1/workspaces/{id}/scheduled-tasks` | POST `{name, prompt, frequency, time_of_day, weekday?, project_id?}`. Times are UTC |
| GET, PATCH, DELETE | `/api/v1/scheduled-tasks/{id}` | Changing the schedule or re-enabling recomputes the next run |
| POST | `/api/v1/scheduled-tasks/{id}/run` | Start a run now; the schedule is unchanged |
| POST | `/api/v1/turns/{id}/interrupt` | Interrupt a running turn (202). The turn ends as `interrupted` |
| PUT, GET | `/api/v1/attachments/{id}/content` | Upload bytes once (uploader only), then download. Size and SHA-256 are checked |
| GET, POST | `/api/v1/workspaces/{id}/connectors` | Connectors are MCP servers. Members read. Create: admin. URL must be http or https |
| GET, PATCH, DELETE | `/api/v1/connectors/{id}` | PATCH `{name?, url?, enabled?}`: admin. `status` is the last startup status codex reported (`starting`, `ready`, `failed`, `cancelled`), or null until a chat starts with it |
| GET, POST | `/api/v1/workspaces/{id}/skills?kind=skill|plugin` | Plugins are skills with `kind: "plugin"` |

Turn creation also accepts `model`, `effort` and `attachment_ids`. Text attachments are
added to the turn input as fenced blocks; other types are refused. Thread creation and
PATCH accept `project_id`. Enabled connectors are passed to each new codex thread as
`mcp_servers`; a connector's status is updated from codex's startup reports.

Turns stay `queued` until the runtime that dispatches them to `codex app-server`
updates them, which is not implemented yet.

## Turn dispatch

Creating a turn returns `201` with status `queued`. A drain task then runs the
thread's queued turns, oldest first, while holding a lock for that thread:

1. Lease the environment's connection: its `endpoint` if set, otherwise a local
   app-server started with `CODEX_HOME=<root>/<environment id>`.
2. On the thread's first turn, send `thread/start` and store the id in
   `threads.codex_thread_id`. Later turns reuse it.
3. Send `turn/start`, store the id in `turns.codex_turn_id`, and mark the turn `running`.
4. Wait for `turn/completed`. Per-call token usage from `thread/tokenUsage/updated` is
   summed into one `usage_records` row.
5. Mark the turn `completed`, `failed` or `interrupted`. Errors from app-server or the
   database are stored on the turn.

Limits: a turn times out after 30 minutes. A lagging event stream fails the turn.

## Browser WebSocket

Connect to `/ws?token=<session token>`. Browsers cannot set headers on WebSocket
upgrades, so the token is in the query string. It can therefore appear in proxy
and access logs; short-lived tickets would be safer.

Client messages:

- `{"type":"subscribe","threadId"}` starts the event stream for a thread you can read.
- `{"type":"unsubscribe","threadId"}`
- `{"type":"approvalResponse","requestId","result"}` answers an `approvalRequest`.
  `result` is the app-server response for that request kind, for example
  `{"decision":"accept"}` for command approvals.

Server messages:

- `{"type":"event","threadId","method","params"}`: app-server notifications, with the
  codex thread id replaced by the gateway thread id.
- `{"type":"approvalRequest","requestId","threadId","method","params"}`
- `{"type":"subscribed"}`, `{"type":"unsubscribed"}`, `{"type":"approvalAccepted"}`,
  `{"type":"error","message"}`

Approvals: while no browser is subscribed to the thread, or after 5 minutes without an
answer, the gateway declines the request so the turn can continue. Requests the
gateway does not know how to answer get a JSON-RPC "method not found" error.

## Layout

```
migrations/            schema, applied in order at startup
src/
├── main.rs         startup: config, pool, migrations, serve
├── lib.rs          router assembly
├── config.rs       environment configuration
├── state.rs        shared AppState
├── error.rs        ApiError → HTTP responses; sqlx error mapping
├── auth.rs         bearer token extractor and token hashing
├── authz.rs        roles and workspace membership checks
├── audit.rs        audit writes and the audit-log endpoint
├── validate.rs     input validation and pagination bounds
├── ws.rs           browser WebSocket: auth, subscriptions, approvals
├── runtime/
│   ├── rpc.rs      JSON-RPC over WebSocket to app-server
│   ├── mod.rs      connection leases, local launch, idle reaper, thread locks
│   ├── hub.rs      routes events and approvals to browser subscribers
│   └── dispatch.rs per-thread turn queue, turn lifecycle, restart recovery
└── routes/         one module per resource
tests/api.rs        REST API tests
tests/runtime.rs    turn dispatch against a mock app-server
tests/realtime.rs   WebSocket, approvals, ordering, recovery, local launch
tests/common/       mock app-server and helpers
```
