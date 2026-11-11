-- Single-use invite links to a workspace. Only a hash of the token is stored.
CREATE TABLE workspace_invites (
    id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id uuid NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    role         text NOT NULL CHECK (role IN ('admin', 'member')),
    token_hash   text NOT NULL UNIQUE,
    created_by   uuid REFERENCES users (id) ON DELETE SET NULL,
    expires_at   timestamptz NOT NULL,
    used_at      timestamptz,
    used_by      uuid REFERENCES users (id) ON DELETE SET NULL,
    created_at   timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX workspace_invites_workspace_idx ON workspace_invites (workspace_id);
