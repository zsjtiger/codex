-- A plugin is a bundle of skills and connectors. Installing it adds those to the workspace, tagged
-- with the plugin; uninstalling (or deleting the plugin) removes the tagged ones.
CREATE TABLE plugins (
    id           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id uuid NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    name         text NOT NULL CHECK (length(name) BETWEEN 1 AND 100),
    description  text NOT NULL DEFAULT '' CHECK (length(description) <= 500),
    -- {"skills": [{"name", "description", "instructions"}], "connectors": [{"name", "url"}]}
    manifest     jsonb NOT NULL,
    created_by   uuid REFERENCES users (id) ON DELETE SET NULL,
    created_at   timestamptz NOT NULL DEFAULT now(),
    UNIQUE (workspace_id, name)
);

ALTER TABLE skills ADD COLUMN plugin_id uuid REFERENCES plugins (id) ON DELETE CASCADE;
ALTER TABLE connectors ADD COLUMN plugin_id uuid REFERENCES plugins (id) ON DELETE CASCADE;
CREATE INDEX skills_plugin ON skills (plugin_id) WHERE plugin_id IS NOT NULL;
CREATE INDEX connectors_plugin ON connectors (plugin_id) WHERE plugin_id IS NOT NULL;
