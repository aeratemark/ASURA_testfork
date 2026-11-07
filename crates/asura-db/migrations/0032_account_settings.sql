-- The account's settings blob: the client's own UI state, kept so the next login can be answered with it.
--
-- The client uploads this blob on the auth connection in two messages that share one request id:
-- AUTH_CMSG_SETTING_UPDATE_SIZE (0x8021) announces the total, then one or more
-- AUTH_CMSG_SETTING_UPDATE_CONTENT (0x8020) carry it in chunks of at most 512 bytes. Measured on this
-- server in `logs/auth.log`: request 5 announced 578 bytes and arrived as 512 + 66, both starting
-- `08 00 66 21`. The same file shows the blob changing length between sessions (454, 455, 457, 578,
-- 579, 581), which is the client saving its own state - the settings array `GW::ui::GetSettings`
-- hands out (Py4GW measured 798 bytes in one session; `docs/UIMANAGER_PORT.md`).
--
-- AUTH_SMSG_ACCOUNT_SETTINGS (0x0016) is answered from this column at login. Until this column
-- existed every login was answered with the one 591-byte blob in
-- `config/Server/account_settings.json`, so the client's upload was acknowledged and then thrown
-- away, and every session started from the same layout.
--
-- NULL means "never uploaded": the login path falls back to the committed blob, which is what a new
-- account gets. The bytes are the client's, not a decoding of them; this schema does not interpret
-- the blob (crates/asura-db/src/account.rs, crates/asura-auth/src/lib.rs).
ALTER TABLE t_account ADD COLUMN IF NOT EXISTS account_settings bytea;
INSERT INTO schema_version (version) VALUES (32) ON CONFLICT (version) DO NOTHING;
