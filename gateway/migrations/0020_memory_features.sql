-- Searching earlier chats for context is opt-in. Off unless the user turns it on.
ALTER TABLE user_settings ADD COLUMN search_chats boolean NOT NULL DEFAULT false;

-- Notes kept for one project. They are added to the project's new turns.
CREATE TABLE project_memories (
    id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id uuid NOT NULL REFERENCES projects (id) ON DELETE CASCADE,
    content    text NOT NULL CHECK (length(content) BETWEEN 1 AND 2000),
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX project_memories_project_idx ON project_memories (project_id, created_at);
