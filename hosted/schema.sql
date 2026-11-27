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
-- Set when a phone opens its pairing page, so the inbox can move on.
ALTER TABLE sessions ADD COLUMN IF NOT EXISTS seen_at timestamptz;
ALTER TABLE accounts ADD COLUMN IF NOT EXISTS display_name text;
ALTER TABLE accounts ALTER COLUMN password_hash DROP NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS accounts_sg_subject ON accounts(sg_subject);
CREATE TABLE IF NOT EXISTS admin_role_snapshot (
 singleton boolean PRIMARY KEY CHECK (singleton),
 subjects jsonb NOT NULL CHECK (jsonb_typeof(subjects)='array'),
 expires_at timestamptz NOT NULL
);
REVOKE ALL ON admin_role_snapshot FROM psms_runtime;
GRANT SELECT ON admin_role_snapshot TO psms_runtime;
CREATE OR REPLACE VIEW archive_quota AS
 SELECT a.id, CASE WHEN EXISTS (
   SELECT 1 FROM admin_role_snapshot s
   WHERE s.expires_at > statement_timestamp() AND s.subjects ? a.sg_subject
 ) THEN NULL::bigint ELSE 1000::bigint END AS message_limit
 FROM accounts a;
REVOKE ALL ON archive_quota FROM psms_runtime;
GRANT SELECT ON archive_quota TO psms_runtime;
CREATE TABLE IF NOT EXISTS browser_access (
 account_id bigint PRIMARY KEY REFERENCES accounts ON DELETE CASCADE,
 digest bytea UNIQUE NOT NULL,
 expires_at timestamptz NOT NULL
);
GRANT SELECT,INSERT,UPDATE,DELETE ON browser_access TO psms_runtime;
COMMIT;
