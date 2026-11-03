-- The portal identity the client presents (M3).
--
-- The login message carries two UUIDs: "who you are" (the portal user id, which the client's portal
-- supplies) and "this session". GWPS stores them as two separate columns on its session table
-- (DbSchema.h, DbSessionColumnsDef: user_id, session_id, account_id) and refuses to serve a character
-- list unless the exact pair resolves. Validation is a lookup on both, so keeping them separate is
-- not a preference: it is what the lookup is.
--
-- Why the column is NOT NULL: the pair *is* the credential, so a session without a portal user is not
-- a weaker session, it is not a session. If a database already holds session rows, this migration
-- fails instead of inventing an identity for them. Backfilling the session id here would make a row
-- that should be refused look valid, and login would start accepting a credential nobody issued --
-- the failure mode would be silent, which is the one thing a login path must not be.
--
-- WHERE portal_user = $1 AND id = $2 needs no new index: id is the primary key, and the lookup returns
-- at most one row. `session_active` (from 0001) already covers the other query, closing an account's
-- older sessions.

ALTER TABLE session ADD COLUMN IF NOT EXISTS portal_user uuid NOT NULL;

-- Record that this migration ran (crates/asura-db/src/migrations.rs; the version is the file number).
INSERT INTO schema_version (version) VALUES (3) ON CONFLICT (version) DO NOTHING;
