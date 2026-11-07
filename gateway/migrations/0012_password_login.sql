-- Accounts can sign in with a username and password. Accounts created before this migration
-- have no username or password and keep working only through the development sign-in.
ALTER TABLE users
    ADD COLUMN username text CHECK (username ~ '^[a-z0-9_.-]{3,32}$'),
    ADD COLUMN password_hash text;
CREATE UNIQUE INDEX users_username_key ON users (username) WHERE username IS NOT NULL;
