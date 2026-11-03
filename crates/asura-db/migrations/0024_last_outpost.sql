-- The last outpost a character was placed in.
--
-- The owner's base rule (docs/22-world-model.md, "Spawn points, and the outpost rule"): every time a
-- character enters the game - at character select, not on a reconnect after a travel - it is placed in the
-- last outpost it visited, even when the client asks for the instance it was standing in. `characters.map_id`
-- cannot answer that: it records the map a character was last in, explorables included.
--
-- Written by the world every time a character is placed in an instance whose type is an outpost
-- (`asura_db::game::remember_outpost`), and read by the auth service when it answers the client's request
-- for an instance. 0 means "never placed in an outpost", which the rule treats as no substitution.
--
-- Idempotent, like every migration here.

ALTER TABLE characters ADD COLUMN IF NOT EXISTS last_outpost int NOT NULL DEFAULT 0;

-- Record that this migration ran (crates/asura-db/src/migrations.rs; the version is the file number).
INSERT INTO schema_version (version) VALUES (24) ON CONFLICT (version) DO NOTHING;
