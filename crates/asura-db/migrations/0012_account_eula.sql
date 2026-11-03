-- The EULA version an account accepted, so the client stops asking after the first time.
--
-- Measured against build 39112 on this server (`.trace-6112.log`, one login):
--
--   server -> client  AUTH_SMSG_ACCOUNT_INFO (0x0011)  the eula byte was `18` = 24, a constant
--   client -> server  AUTH_CMSG_ACCEPT_EULA  (0x8026)  the byte was `1a` = 26, after the client
--                                                      showed the page and the OK was clicked
--
-- So the byte is a version: the client shows the EULA while the number the server reports is lower
-- than the one it knows, and tells the server which version it accepted when a person accepts. This
-- server answered 24 on every login, which is why the page appeared on every login.
--
-- 0 means never accepted, which is below any version the client knows, so a fresh account is asked
-- once. `smallint` matches the one-byte wire field and leaves room for the number to move.

ALTER TABLE account ADD COLUMN IF NOT EXISTS eula_accepted smallint NOT NULL DEFAULT 0;

-- Record that this migration ran (crates/asura-db/src/migrations.rs; the version is the file number).
INSERT INTO schema_version (version) VALUES (12) ON CONFLICT (version) DO NOTHING;
