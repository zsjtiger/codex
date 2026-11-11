-- Sign-in state of OAuth connectors. The grant itself is kept by codex on the runtime host.
ALTER TABLE connectors ADD COLUMN oauth_state text CHECK (oauth_state IN ('pending', 'signed_in', 'failed'));
ALTER TABLE connectors ADD COLUMN oauth_error text CHECK (length(oauth_error) <= 1000);
ALTER TABLE connectors ADD COLUMN oauth_updated_at timestamptz;
