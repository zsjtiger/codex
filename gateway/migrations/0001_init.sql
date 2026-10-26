-- Accounts and sessions -----------------------------------------------------

CREATE TABLE users (
    id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    email        text NOT NULL UNIQUE CHECK (email = lower(email) AND length(email) BETWEEN 3 AND 320),
    display_name text NOT NULL CHECK (length(display_name) BETWEEN 1 AND 100),
    created_at   timestamptz NOT NULL DEFAULT now(),
    updated_at   timestamptz NOT NULL DEFAULT now()
);

-- Only a SHA-256 of the bearer token is stored, never the token itself.
CREATE TABLE sessions (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id     uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    token_hash  text NOT NULL UNIQUE CHECK (token_hash ~ '^[0-9a-f]{64}$'),
    created_at  timestamptz NOT NULL DEFAULT now(),
    expires_at  timestamptz NOT NULL,
    revoked_at  timestamptz
);
CREATE INDEX sessions_user_id_idx ON sessions (user_id);

-- Tenants, membership and environments -------------------------------------

CREATE TABLE workspaces (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    slug        text NOT NULL UNIQUE CHECK (slug ~ '^[a-z0-9]([a-z0-9-]{1,38}[a-z0-9])?$'),
    name        text NOT NULL CHECK (length(name) BETWEEN 1 AND 100),
    created_by  uuid REFERENCES users (id) ON DELETE SET NULL,
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE workspace_members (
    workspace_id uuid NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    user_id      uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    role         text NOT NULL CHECK (role IN ('owner', 'admin', 'member')),
    created_at   timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, user_id)
);
CREATE INDEX workspace_members_user_id_idx ON workspace_members (user_id);
-- Exactly one owner per workspace.
CREATE UNIQUE INDEX workspace_members_one_owner_idx ON workspace_members (workspace_id) WHERE role = 'owner';

-- A runtime that executes threads (for example one codex app-server instance).
CREATE TABLE environments (
    id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id uuid NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    name         text NOT NULL CHECK (length(name) BETWEEN 1 AND 100),
    status       text NOT NULL DEFAULT 'provisioning'
                 CHECK (status IN ('provisioning', 'ready', 'stopped', 'failed')),
    endpoint     text CHECK (endpoint ~ '^wss?://'),
    created_at   timestamptz NOT NULL DEFAULT now(),
    updated_at   timestamptz NOT NULL DEFAULT now(),
    UNIQUE (workspace_id, name)
);

-- Conversations ------------------------------------------------------------

CREATE TABLE threads (
    id               uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id     uuid NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    environment_id   uuid REFERENCES environments (id) ON DELETE SET NULL,
    created_by       uuid REFERENCES users (id) ON DELETE SET NULL,
    title            text NOT NULL DEFAULT 'Untitled' CHECK (length(title) BETWEEN 1 AND 200),
    state            text NOT NULL DEFAULT 'idle' CHECK (state IN ('idle', 'running', 'failed')),
    -- Thread id assigned by codex app-server; NULL until the thread is started there.
    codex_thread_id  text UNIQUE,
    archived_at      timestamptz,
    created_at       timestamptz NOT NULL DEFAULT now(),
    updated_at       timestamptz NOT NULL DEFAULT now(),
    last_activity_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX threads_workspace_activity_idx ON threads (workspace_id, last_activity_at DESC);

CREATE TABLE turns (
    id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    thread_id    uuid NOT NULL REFERENCES threads (id) ON DELETE CASCADE,
    seq          integer NOT NULL CHECK (seq > 0),
    status       text NOT NULL DEFAULT 'queued'
                 CHECK (status IN ('queued', 'running', 'completed', 'failed', 'interrupted')),
    input        text NOT NULL CHECK (length(input) BETWEEN 1 AND 100000),
    error        text,
    created_by   uuid REFERENCES users (id) ON DELETE SET NULL,
    created_at   timestamptz NOT NULL DEFAULT now(),
    completed_at timestamptz,
    UNIQUE (thread_id, seq)
);

-- Files ---------------------------------------------------------------------

-- Metadata only; the bytes live in object storage under storage_key.
CREATE TABLE attachments (
    id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id uuid NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    thread_id    uuid REFERENCES threads (id) ON DELETE CASCADE,
    uploaded_by  uuid REFERENCES users (id) ON DELETE SET NULL,
    filename     text NOT NULL CHECK (length(filename) BETWEEN 1 AND 255),
    content_type text NOT NULL CHECK (length(content_type) BETWEEN 3 AND 255),
    size_bytes   bigint NOT NULL CHECK (size_bytes >= 0),
    sha256       text CHECK (sha256 ~ '^[0-9a-f]{64}$'),
    storage_key  text NOT NULL UNIQUE,
    created_at   timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX attachments_workspace_idx ON attachments (workspace_id);
CREATE INDEX attachments_thread_idx ON attachments (thread_id);

-- Accounting and audit -----------------------------------------------------

-- Written by the runtime when a turn finishes; read by the usage API.
CREATE TABLE usage_records (
    id            bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    workspace_id  uuid NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    user_id       uuid REFERENCES users (id) ON DELETE SET NULL,
    thread_id     uuid REFERENCES threads (id) ON DELETE SET NULL,
    turn_id       uuid REFERENCES turns (id) ON DELETE SET NULL,
    input_tokens  bigint NOT NULL DEFAULT 0 CHECK (input_tokens >= 0),
    output_tokens bigint NOT NULL DEFAULT 0 CHECK (output_tokens >= 0),
    recorded_at   timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX usage_records_workspace_time_idx ON usage_records (workspace_id, recorded_at);

CREATE TABLE audit_logs (
    id           bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    workspace_id uuid REFERENCES workspaces (id) ON DELETE CASCADE,
    actor_id     uuid REFERENCES users (id) ON DELETE SET NULL,
    action       text NOT NULL,
    target_type  text NOT NULL,
    target_id    text NOT NULL,
    metadata     jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at   timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX audit_logs_workspace_time_idx ON audit_logs (workspace_id, created_at DESC);
