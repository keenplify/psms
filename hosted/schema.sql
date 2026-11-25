BEGIN;
CREATE TABLE IF NOT EXISTS accounts (
 id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
 username text UNIQUE NOT NULL,
 password_hash text NOT NULL
);
CREATE TABLE IF NOT EXISTS sessions (
 digest bytea PRIMARY KEY, account_id bigint NOT NULL REFERENCES accounts ON DELETE CASCADE,
 expires_at timestamptz NOT NULL, scope text NOT NULL CHECK (scope IN ('owner','upload'))
);
CREATE INDEX IF NOT EXISTS sessions_by_account ON sessions(account_id);
CREATE TABLE IF NOT EXISTS archived_messages (
 account_id bigint NOT NULL REFERENCES accounts ON DELETE CASCADE, id text NOT NULL,
 sender text NOT NULL, body text NOT NULL, contact_id text,
 received_at timestamptz NOT NULL DEFAULT now(),
 PRIMARY KEY (account_id,id)
);
CREATE INDEX IF NOT EXISTS archive_order ON archived_messages(account_id,received_at DESC,id);
ALTER TABLE accounts ADD COLUMN IF NOT EXISTS sg_subject text;
ALTER TABLE accounts ADD COLUMN IF NOT EXISTS display_name text;
ALTER TABLE accounts ALTER COLUMN password_hash DROP NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS accounts_sg_subject ON accounts(sg_subject);
COMMIT;
