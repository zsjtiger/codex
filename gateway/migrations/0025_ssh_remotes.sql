-- Repositories reached over SSH. The gateway makes the key; its private half is encrypted, and the
-- public half is shown so it can be added as a deploy key. The host key is pinned at creation.
-- A repository is ready once its key has cloned it; sessions start only on ready repositories.
ALTER TABLE code_repos ADD COLUMN ssh_key_ciphertext bytea;
ALTER TABLE code_repos ADD COLUMN ssh_key_nonce bytea;
ALTER TABLE code_repos ADD COLUMN ssh_public_key text CHECK (char_length(ssh_public_key) <= 2000);
ALTER TABLE code_repos ADD COLUMN ssh_known_hosts text CHECK (char_length(ssh_known_hosts) <= 4000);
ALTER TABLE code_repos ADD COLUMN ssh_fingerprint text CHECK (char_length(ssh_fingerprint) <= 2000);
ALTER TABLE code_repos ADD COLUMN ready boolean NOT NULL DEFAULT true;
