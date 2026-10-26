-- Connectors are MCP servers. Enabled connectors are passed to codex as `mcp_servers`
-- when a thread starts.
CREATE TABLE connectors (
    id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id uuid NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    name         text NOT NULL CHECK (length(name) BETWEEN 1 AND 100),
    -- The server key in codex's config, derived from the name.
    key          text NOT NULL CHECK (key ~ '^[a-z0-9_-]{1,40}$'),
    url          text NOT NULL CHECK (url ~ '^https?://' AND length(url) <= 2000),
    enabled      boolean NOT NULL DEFAULT true,
    created_by   uuid REFERENCES users (id) ON DELETE SET NULL,
    created_at   timestamptz NOT NULL DEFAULT now(),
    updated_at   timestamptz NOT NULL DEFAULT now(),
    UNIQUE (workspace_id, key)
);

-- Plugins are skill-style instruction bundles. They share the skills table.
ALTER TABLE skills ADD COLUMN kind text NOT NULL DEFAULT 'skill' CHECK (kind IN ('skill', 'plugin'));
