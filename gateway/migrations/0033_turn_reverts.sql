-- When a turn was reverted: codex rewound the chat to before it. The turn and the ones after it stay in the
-- history, marked, so what happened is still visible.
ALTER TABLE turns ADD COLUMN reverted_at timestamptz;
