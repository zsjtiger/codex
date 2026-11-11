-- GitHub repositories linked to a workspace for code sessions. The token, if any, is
-- encrypted with GATEWAY_SECRET_KEY; it is never returned by the API.
CREATE TABLE code_repos (
    id               uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id     uuid NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    full_name        text NOT NULL CHECK (char_length(full_name) BETWEEN 3 AND 200),
    default_branch   text NOT NULL DEFAULT 'main' CHECK (char_length(default_branch) BETWEEN 1 AND 200),
    token_ciphertext bytea,
    token_nonce      bytea,
    created_by       uuid REFERENCES users(id) ON DELETE SET NULL,
    created_at       timestamptz NOT NULL DEFAULT now(),
    last_synced_at   timestamptz,
    UNIQUE (workspace_id, full_name),
    CHECK ((token_ciphertext IS NULL) = (token_nonce IS NULL))
);

-- A code thread works in its own git worktree on its own branch.
ALTER TABLE threads ADD COLUMN repo_id uuid REFERENCES code_repos(id) ON DELETE SET NULL;
ALTER TABLE threads ADD COLUMN work_branch text;
