-- The unified diff of everything a code turn changed, as codex reports it (turn/diff/updated). Unlike the
-- per-file changes in turn_work, it also covers files changed by shell commands. Each report replaces the last.
CREATE TABLE turn_diffs (
    thread_id      uuid NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    codex_turn_id  text NOT NULL,
    diff           text NOT NULL,
    updated_at     timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (thread_id, codex_turn_id)
);
