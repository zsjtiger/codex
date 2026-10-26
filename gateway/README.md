# gateway

Rust + axum backend for the cloud Codex web app. It is the control plane between the
browser and per-session `codex app-server` processes (see `../codex-rs/app-server`).

## Status

- REST control plane under `/api/v1`: sessions, workspaces, members, environments,
  threads, turns, attachments, usage and audit log. Backed by Postgres.
- `GET /healthz` returns `ok`.
- `GET /ws` is still an echo placeholder. It is not yet authenticated or connected to
  `codex app-server`.

Not implemented yet: real sign-in (only the dev login exists), dispatching turns to
`codex app-server`, object storage for attachment bytes, and rate limiting.

## Configuration

| Variable          | Default           | Meaning                                                    |
|-------------------|-------------------|------------------------------------------------------------|
| `DATABASE_URL`    | required          | Postgres connection string                                 |
| `GATEWAY_ADDR`    | `127.0.0.1:8080`  | Listen address                                             |
| `GATEWAY_DEV_LOGIN` | unset           | `1` or `true` enables `POST /api/v1/auth/sessions`. Dev only. |

Migrations in `migrations/` run automatically at startup.

## Run and test

```sh
export DATABASE_URL=postgres://postgres@127.0.0.1:5432/codex_gateway
export GATEWAY_DEV_LOGIN=1
cargo run                        # applies migrations, then listens on 127.0.0.1:8080
cargo test                       # integration tests; sqlx creates a fresh database per test
cargo clippy --all-targets
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

Turns stay `queued` until the runtime that dispatches them to `codex app-server`
updates them, which is not implemented yet.

## Layout

```
migrations/0001_init.sql   schema
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
├── ws.rs           /ws placeholder
└── routes/         one module per resource
tests/api.rs        end-to-end tests
```
