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
-- Set on a phone token's first stored message, so its setup page can confirm it worked.
ALTER TABLE sessions ADD COLUMN IF NOT EXISTS used_at timestamptz;
-- Set when the person finishes the phone's setup page, so the inbox shows the recovery kit.
ALTER TABLE sessions ADD COLUMN IF NOT EXISTS ready_at timestamptz;
-- Last message a phone token stored, for the phone list in Settings.
ALTER TABLE sessions ADD COLUMN IF NOT EXISTS last_used_at timestamptz;
-- Read status is shared by every signed-in browser. It reveals nothing about message contents.
ALTER TABLE archived_messages ADD COLUMN IF NOT EXISTS read_at timestamptz;
ALTER TABLE archived_messages ADD COLUMN IF NOT EXISTS starred boolean NOT NULL DEFAULT false;
-- Contact cards, sealed with the archive key like messages. The phone's copy comes from its
-- address book; the manual copy holds edits made in an inbox and wins when both exist.
CREATE TABLE IF NOT EXISTS contact_cards (
 account_id bigint NOT NULL REFERENCES accounts ON DELETE CASCADE,
 contact_id text NOT NULL CHECK (contact_id ~ '^[0-9a-f]{64}$'),
 phone_card text, phone_updated_at timestamptz,
 manual_card text, manual_updated_at timestamptz,
 PRIMARY KEY (account_id, contact_id)
);
GRANT SELECT,INSERT,UPDATE,DELETE ON contact_cards TO psms_runtime;
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
-- Plans set each account's limits. NULL means unlimited. Billing will set accounts.plan;
-- the runtime role can read plans but never change them.
CREATE TABLE IF NOT EXISTS plans (
 id text PRIMARY KEY CHECK (id ~ '^[a-z]+$'), name text NOT NULL,
 message_limit bigint CHECK (message_limit > 0), api_calls_per_30_min integer CHECK (api_calls_per_30_min > 0),
 monthly_price_cents integer NOT NULL DEFAULT 0, available boolean NOT NULL DEFAULT false
);
INSERT INTO plans(id,name,message_limit,api_calls_per_30_min,monthly_price_cents,available) VALUES
 ('free','Free',100,100,0,true),
 ('plus','Plus',NULL,NULL,0,false),
 ('enterprise','Enterprise',NULL,NULL,0,false)
ON CONFLICT(id) DO NOTHING;
REVOKE ALL ON plans FROM psms_runtime;
GRANT SELECT ON plans TO psms_runtime;
-- Paid features. Hosted webhooks, when they arrive, are for plans with this flag.
ALTER TABLE plans ADD COLUMN IF NOT EXISTS webhooks boolean NOT NULL DEFAULT false;
UPDATE plans SET webhooks=true WHERE id IN ('plus','enterprise') AND NOT webhooks;
ALTER TABLE accounts ADD COLUMN IF NOT EXISTS plan text NOT NULL DEFAULT 'free' REFERENCES plans;
-- Plan details admins edit from the inbox: price wording, perks shown on the pricing page,
-- and feature gates. NULL limits mean unlimited.
ALTER TABLE plans ADD COLUMN IF NOT EXISTS price_label text;
ALTER TABLE plans ADD COLUMN IF NOT EXISTS perks text[] NOT NULL DEFAULT '{}';
ALTER TABLE plans ADD COLUMN IF NOT EXISTS max_phones integer CHECK (max_phones > 0);
ALTER TABLE plans ADD COLUMN IF NOT EXISTS replies boolean NOT NULL DEFAULT true;
ALTER TABLE plans ADD COLUMN IF NOT EXISTS autofill boolean NOT NULL DEFAULT true;
ALTER TABLE plans ADD COLUMN IF NOT EXISTS sort integer NOT NULL DEFAULT 0;
UPDATE plans SET sort=0, max_phones=3, perks=ARRAY['Your latest 100 encrypted messages; the oldest unstarred ones make room for new texts','Up to 3 phones','Reply from anywhere with pSMS for Mac','Contact cards, read status and stars','Browser autofill for one-time codes','100 API calls every 30 minutes'] WHERE id='free' AND perks='{}';
UPDATE plans SET sort=1, monthly_price_cents=300, price_label='Coming soon', perks=ARRAY['Unlimited messages, kept until you delete them','Unlimited phones and Macs','Unlimited API calls','Webhooks when a text arrives','Everything in Free'] WHERE id='plus' AND perks='{}';
UPDATE plans SET sort=2, price_label='Contact us', perks=ARRAY['Everything in Plus','Team inboxes for shared numbers','Custom retention and data residency','Priority support'] WHERE id='enterprise' AND perks='{}';
GRANT UPDATE (name, message_limit, api_calls_per_30_min, monthly_price_cents, available, webhooks,
  price_label, perks, max_phones, replies, autofill) ON plans TO psms_runtime;
-- Admins move accounts between plans by hand from the inbox.
GRANT UPDATE (plan) ON accounts TO psms_runtime;
-- Effective limits for each account. Admins are unlimited whatever their plan.
CREATE OR REPLACE VIEW archive_quota AS
 SELECT a.id, CASE WHEN admin.yes THEN NULL::bigint ELSE p.message_limit END AS message_limit,
   p.id AS plan, p.name AS plan_name,
   CASE WHEN admin.yes THEN NULL::integer ELSE p.api_calls_per_30_min END AS api_calls_per_30_min,
   admin.yes OR p.webhooks AS webhooks,
   CASE WHEN admin.yes THEN NULL::integer ELSE p.max_phones END AS max_phones,
   admin.yes OR p.replies AS replies,
   admin.yes OR p.autofill AS autofill,
   admin.yes AS admin
 FROM accounts a JOIN plans p ON p.id=a.plan
 CROSS JOIN LATERAL (SELECT EXISTS (
   SELECT 1 FROM admin_role_snapshot s
   WHERE s.expires_at > statement_timestamp() AND s.subjects ? a.sg_subject
 ) AS yes) admin;
REVOKE ALL ON archive_quota FROM psms_runtime;
GRANT SELECT ON archive_quota TO psms_runtime;
CREATE TABLE IF NOT EXISTS browser_access (
 account_id bigint PRIMARY KEY REFERENCES accounts ON DELETE CASCADE,
 digest bytea UNIQUE NOT NULL,
 expires_at timestamptz NOT NULL
);
GRANT SELECT,INSERT,UPDATE,DELETE ON browser_access TO psms_runtime;
ALTER TABLE sessions DROP CONSTRAINT IF EXISTS sessions_scope_check;
ALTER TABLE sessions ADD CONSTRAINT sessions_scope_check CHECK (scope IN ('owner','upload','computer'));
-- Texts queued in an inbox for pSMS for Mac (or another sending device), sealed with the
-- archive key. The envelope is dropped once the device reports the text sent.
CREATE TABLE IF NOT EXISTS outbox (
 id text NOT NULL, account_id bigint NOT NULL REFERENCES accounts ON DELETE CASCADE,
 envelope text, status text NOT NULL DEFAULT 'queued' CHECK (status IN ('queued','sending','sent','failed')),
 error text, created_at timestamptz NOT NULL DEFAULT now(), claimed_at timestamptz, finished_at timestamptz,
 PRIMARY KEY (account_id, id)
);
GRANT SELECT,INSERT,UPDATE,DELETE ON outbox TO psms_runtime;
-- API calls from tokens other services use, counted per free account in 30-minute windows.
CREATE TABLE IF NOT EXISTS api_usage (
 account_id bigint PRIMARY KEY REFERENCES accounts ON DELETE CASCADE,
 window_start timestamptz NOT NULL, calls integer NOT NULL
);
GRANT SELECT,INSERT,UPDATE,DELETE ON api_usage TO psms_runtime;
COMMIT;
