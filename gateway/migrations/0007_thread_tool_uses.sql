-- Tool, search, and file actions Codex completed in a thread, shown in the conversation's Used panel.
CREATE TABLE thread_tool_uses (
    id         uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    thread_id  uuid NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    item_id    text NOT NULL CHECK (length(item_id) BETWEEN 1 AND 200),
    kind       text NOT NULL CHECK (length(kind) <= 40),
    label      text NOT NULL CHECK (length(label) <= 500),
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (thread_id, item_id)
);
CREATE INDEX thread_tool_uses_thread_idx ON thread_tool_uses (thread_id, created_at, id);
