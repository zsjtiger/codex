-- A GitHub account connected by a user through OAuth. The token is encrypted with GATEWAY_SECRET_KEY.
CREATE TABLE github_accounts (
    user_id          uuid PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    login            text NOT NULL,
    token_ciphertext bytea NOT NULL,
    token_nonce      bytea NOT NULL,
    connected_at     timestamptz NOT NULL DEFAULT now()
);

-- One-time states for the OAuth round trip. Each binds a callback to the user who started it.
CREATE TABLE github_oauth_states (
    state      text PRIMARY KEY,
    user_id    uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now()
);
