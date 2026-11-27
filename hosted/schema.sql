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
-- Plus features for self-hosted archives and teams.
ALTER TABLE plans ADD COLUMN IF NOT EXISTS self_host_sync boolean NOT NULL DEFAULT false;
ALTER TABLE plans ADD COLUMN IF NOT EXISTS teams boolean NOT NULL DEFAULT false;
ALTER TABLE plans ADD COLUMN IF NOT EXISTS pro_features_seeded boolean NOT NULL DEFAULT false;
UPDATE plans SET self_host_sync=true, teams=true, pro_features_seeded=true WHERE id IN ('plus','enterprise') AND NOT pro_features_seeded;
UPDATE plans SET pro_features_seeded=true WHERE NOT pro_features_seeded;
GRANT UPDATE (name, message_limit, api_calls_per_30_min, monthly_price_cents, available, webhooks,
  price_label, perks, max_phones, replies, autofill, self_host_sync, teams) ON plans TO psms_runtime;
-- Admins move accounts between plans by hand from the inbox.
-- Enterprise capacity is set per account; it overrides the plan's message limit.
ALTER TABLE accounts ADD COLUMN IF NOT EXISTS message_limit_override bigint CHECK (message_limit_override > 0);
GRANT UPDATE (plan, message_limit_override) ON accounts TO psms_runtime;
-- Plus keeps the latest 5,000 messages (set once; admins can change it after).
ALTER TABLE plans ADD COLUMN IF NOT EXISTS capacity_seeded boolean NOT NULL DEFAULT false;
UPDATE plans SET message_limit=5000,
  perks=array_replace(perks,'Unlimited messages, kept until you delete them','Your latest 5,000 messages; the oldest unstarred ones make room for new texts')
  WHERE id='plus' AND NOT capacity_seeded;
UPDATE plans SET perks=array_replace(perks,'Everything in Plus','Everything in Plus, with the message capacity you need')
  WHERE id='enterprise' AND NOT capacity_seeded;
UPDATE plans SET capacity_seeded=true WHERE NOT capacity_seeded;
-- Plus perks for self-hosted archives, teams and managed webhooks (set once; admins edit after).
ALTER TABLE plans ADD COLUMN IF NOT EXISTS pro_perks_seeded boolean NOT NULL DEFAULT false;
UPDATE plans SET name='Plus', perks=ARRAY[
  'Your latest 5,000 messages; the oldest unstarred ones make room for new texts',
  'Connect a self-hosted archive: phones need no tunnel, with a sealed off-site backup and restore',
  'Read and reply from any browser, even while your Mac is off; replies send when it is back',
  'Team inboxes for shared numbers',
  'Managed webhooks with retries, a delivery log and replay',
  'Unlimited phones, Macs and API calls',
  'Everything in Free'] WHERE id='plus' AND NOT pro_perks_seeded;
UPDATE plans SET perks=ARRAY['Everything in Plus','Message capacity sized to your team','Priority support'] WHERE id='enterprise' AND NOT pro_perks_seeded;
UPDATE plans SET pro_perks_seeded=true WHERE NOT pro_perks_seeded;
-- The paid plan is called Plus.
UPDATE plans SET name='Plus' WHERE id='plus' AND name='Pro';
UPDATE plans SET perks=array_replace(perks,'Everything in Pro','Everything in Plus');
-- Effective limits for each account. Admins are unlimited whatever their plan.
CREATE OR REPLACE VIEW archive_quota AS
 SELECT a.id, CASE WHEN admin.yes THEN NULL::bigint ELSE coalesce(a.message_limit_override, p.message_limit) END AS message_limit,
   p.id AS plan, p.name AS plan_name,
   CASE WHEN admin.yes THEN NULL::integer ELSE p.api_calls_per_30_min END AS api_calls_per_30_min,
   admin.yes OR p.webhooks AS webhooks,
   CASE WHEN admin.yes THEN NULL::integer ELSE p.max_phones END AS max_phones,
   admin.yes OR p.replies AS replies,
   admin.yes OR p.autofill AS autofill,
   admin.yes AS admin,
   admin.yes OR p.self_host_sync AS self_host_sync,
   admin.yes OR p.teams AS teams
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
-- Team inboxes: members open the owner's inbox (they need its recovery words to read it).
CREATE TABLE IF NOT EXISTS account_members (
 owner_id bigint NOT NULL REFERENCES accounts ON DELETE CASCADE,
 member_id bigint NOT NULL REFERENCES accounts ON DELETE CASCADE,
 added_at timestamptz NOT NULL DEFAULT now(),
 PRIMARY KEY (owner_id, member_id), CHECK (owner_id <> member_id)
);
CREATE INDEX IF NOT EXISTS account_members_by_member ON account_members(member_id);
GRANT SELECT,INSERT,DELETE ON account_members TO psms_runtime;
-- Managed webhooks: Papamo delivers message events with retries and keeps a delivery log.
CREATE TABLE IF NOT EXISTS hosted_webhooks (
 id text PRIMARY KEY, account_id bigint NOT NULL REFERENCES accounts ON DELETE CASCADE,
 url text NOT NULL, secret text NOT NULL, created_at timestamptz NOT NULL DEFAULT now(),
 UNIQUE (account_id, url)
);
GRANT SELECT,INSERT,UPDATE,DELETE ON hosted_webhooks TO psms_runtime;
CREATE TABLE IF NOT EXISTS hosted_webhook_deliveries (
 id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
 webhook_id text NOT NULL REFERENCES hosted_webhooks ON DELETE CASCADE,
 account_id bigint NOT NULL, message_id text NOT NULL, event text NOT NULL,
 status text NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','delivered','failed')),
 attempts integer NOT NULL DEFAULT 0, due timestamptz NOT NULL DEFAULT now(),
 last_status integer, last_error text, created_at timestamptz NOT NULL DEFAULT now(), delivered_at timestamptz
);
CREATE INDEX IF NOT EXISTS hosted_webhook_due ON hosted_webhook_deliveries(due) WHERE status='pending';
CREATE INDEX IF NOT EXISTS hosted_webhook_log ON hosted_webhook_deliveries(account_id, id DESC);
GRANT SELECT,INSERT,UPDATE,DELETE ON hosted_webhook_deliveries TO psms_runtime;
CREATE OR REPLACE FUNCTION queue_hosted_webhooks() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  INSERT INTO hosted_webhook_deliveries(webhook_id, account_id, message_id, event)
  SELECT w.id, w.account_id, NEW.id, 'message.created' FROM hosted_webhooks w
  JOIN archive_quota q ON q.id = w.account_id
  WHERE w.account_id = NEW.account_id AND q.webhooks;
  RETURN NEW;
END $$;
DROP TRIGGER IF EXISTS archived_messages_webhooks ON archived_messages;
CREATE TRIGGER archived_messages_webhooks AFTER INSERT ON archived_messages
 FOR EACH ROW EXECUTE FUNCTION queue_hosted_webhooks();
-- Cards sent before the fixed pSMS shortcut (27 Nov 2026) may lack the name and photo; ask
-- phones for them again. Re-running changes nothing new.
UPDATE contact_cards SET phone_updated_at = NULL
 WHERE phone_updated_at < '2026-11-27 09:00:00+00';
COMMIT;
-- Photos and other files that came with a text, sealed with the archive key like the message.
-- pSMS for Mac uploads them after their message; they are deleted with it.
CREATE TABLE IF NOT EXISTS message_attachments (
 account_id bigint NOT NULL, message_id text NOT NULL,
 idx smallint NOT NULL CHECK (idx BETWEEN 0 AND 9),
 envelope text NOT NULL, created_at timestamptz NOT NULL DEFAULT now(),
 PRIMARY KEY (account_id, message_id, idx),
 FOREIGN KEY (account_id, message_id) REFERENCES archived_messages(account_id, id) ON DELETE CASCADE
);
GRANT SELECT,INSERT,UPDATE,DELETE ON message_attachments TO psms_runtime;
-- Conversations and reactions, end-to-end encrypted like messages. A chat's name, members
-- and Messages identifier are sealed; the server only links rows by an opaque chat ID.
ALTER TABLE archived_messages ADD COLUMN IF NOT EXISTS chat_id text CHECK (chat_id ~ '^[0-9a-f]{32}$');
CREATE INDEX IF NOT EXISTS archive_chat ON archived_messages(account_id,chat_id,received_at DESC) WHERE chat_id IS NOT NULL;
CREATE TABLE IF NOT EXISTS chats (
 account_id bigint NOT NULL REFERENCES accounts ON DELETE CASCADE,
 id text NOT NULL CHECK (id ~ '^[0-9a-f]{32}$'),
 envelope text NOT NULL, updated_at timestamptz NOT NULL DEFAULT now(),
 PRIMARY KEY (account_id, id)
);
GRANT SELECT,INSERT,UPDATE,DELETE ON chats TO psms_runtime;
CREATE TABLE IF NOT EXISTS message_reactions (
 account_id bigint NOT NULL, id text NOT NULL, message_id text NOT NULL,
 envelope text NOT NULL, created_at timestamptz NOT NULL DEFAULT now(),
 PRIMARY KEY (account_id, id),
 FOREIGN KEY (account_id, message_id) REFERENCES archived_messages(account_id, id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS message_reactions_message ON message_reactions(account_id,message_id);
GRANT SELECT,INSERT,UPDATE,DELETE ON message_reactions TO psms_runtime;
