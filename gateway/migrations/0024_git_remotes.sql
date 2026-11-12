-- Repositories added by git address, for any git host. The address and the username its token is used
-- with are stored on the repository; the token itself is encrypted, like a GitHub token.
ALTER TABLE code_repos ADD COLUMN clone_url text CHECK (char_length(clone_url) <= 500);
ALTER TABLE code_repos ADD COLUMN username text CHECK (char_length(username) <= 200);
