-- Last startup status codex reported for this connector's MCP server, in any thread of the workspace.
ALTER TABLE connectors ADD COLUMN status text CHECK (status IN ('starting', 'ready', 'failed', 'cancelled'));
ALTER TABLE connectors ADD COLUMN status_error text CHECK (length(status_error) <= 2000);
ALTER TABLE connectors ADD COLUMN status_at timestamptz;
