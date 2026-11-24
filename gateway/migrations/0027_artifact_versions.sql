-- The title and content an artifact had before each change, with who made the change. Kept so an
-- earlier version can be seen and restored.
CREATE TABLE artifact_versions (
    id          bigserial PRIMARY KEY,
    artifact_id uuid NOT NULL REFERENCES artifacts(id) ON DELETE CASCADE,
    title       text NOT NULL,
    content     text NOT NULL,
    replaced_by uuid REFERENCES users(id) ON DELETE SET NULL,
    replaced_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX artifact_versions_artifact ON artifact_versions (artifact_id, id);
