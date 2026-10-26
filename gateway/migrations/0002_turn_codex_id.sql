-- Turn id assigned by codex app-server, used to match its notifications to a turn.
ALTER TABLE turns ADD COLUMN codex_turn_id text;
CREATE UNIQUE INDEX turns_codex_turn_id_idx ON turns (codex_turn_id) WHERE codex_turn_id IS NOT NULL;
