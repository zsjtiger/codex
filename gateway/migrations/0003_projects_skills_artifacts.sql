-- Projects group chats and carry instructions that apply to their threads.
CREATE TABLE projects (
    id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id uuid NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    name         text NOT NULL CHECK (length(name) BETWEEN 1 AND 100),
    instructions text NOT NULL DEFAULT '' CHECK (length(instructions) <= 20000),
    created_by   uuid REFERENCES users (id) ON DELETE SET NULL,
    created_at   timestamptz NOT NULL DEFAULT now(),
    updated_at   timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX projects_workspace_idx ON projects (workspace_id);

ALTER TABLE threads ADD COLUMN project_id uuid REFERENCES projects (id) ON DELETE SET NULL;
CREATE INDEX threads_project_idx ON threads (project_id);

-- Per-turn settings passed to app-server.
ALTER TABLE turns ADD COLUMN model text CHECK (length(model) BETWEEN 1 AND 100);
ALTER TABLE turns ADD COLUMN effort text CHECK (length(effort) BETWEEN 1 AND 32);

-- Skills are workspace instructions that are added to every new codex thread.
CREATE TABLE skills (
    id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id uuid NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    name         text NOT NULL CHECK (length(name) BETWEEN 1 AND 100),
    description  text NOT NULL DEFAULT '' CHECK (length(description) <= 500),
    instructions text NOT NULL CHECK (length(instructions) BETWEEN 1 AND 20000),
    enabled      boolean NOT NULL DEFAULT true,
    created_by   uuid REFERENCES users (id) ON DELETE SET NULL,
    created_at   timestamptz NOT NULL DEFAULT now(),
    updated_at   timestamptz NOT NULL DEFAULT now(),
    UNIQUE (workspace_id, name)
);

CREATE TABLE artifacts (
    id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id uuid NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    project_id   uuid REFERENCES projects (id) ON DELETE SET NULL,
    thread_id    uuid REFERENCES threads (id) ON DELETE SET NULL,
    title        text NOT NULL CHECK (length(title) BETWEEN 1 AND 200),
    kind         text NOT NULL CHECK (kind IN ('document', 'slides', 'design', 'dashboard')),
    content      text NOT NULL DEFAULT '' CHECK (length(content) <= 500000),
    created_by   uuid REFERENCES users (id) ON DELETE SET NULL,
    created_at   timestamptz NOT NULL DEFAULT now(),
    updated_at   timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX artifacts_workspace_idx ON artifacts (workspace_id, updated_at DESC);

-- Set once the file bytes have been stored.
ALTER TABLE attachments ADD COLUMN uploaded boolean NOT NULL DEFAULT false;
