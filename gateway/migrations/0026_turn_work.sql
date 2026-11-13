-- Finished commands and file changes of a code turn, as the app-server reported them. Kept so the work
-- log can be shown again after a reload. Large outputs and diffs are capped when they are stored.
CREATE TABLE turn_work (
    thread_id      uuid NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    codex_turn_id  text NOT NULL,
    item_id        text NOT NULL,
    seq            bigserial NOT NULL,
    item           jsonb NOT NULL,
    PRIMARY KEY (thread_id, item_id)
);
CREATE INDEX turn_work_thread_seq ON turn_work (thread_id, seq);
