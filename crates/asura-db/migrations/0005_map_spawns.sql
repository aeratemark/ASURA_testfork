-- Where a character stands when it enters a map (M4).
--
-- `map_config` has held the map id, its type and its blob file id since 0001 -- enough to *identify* a
-- map, not enough to put a character in it. A client is told two things on entering: which blob to
-- load (`INSTANCE_LOAD_SPAWN_POINT.map_file_id`) and where inside it to place the agent. Without the
-- second the character stands at (0,0), which on every pre-Searing map is outside the walkable
-- geometry, and the client either drops the character or leaves the loading screen stuck.
--
-- The values seeded for the development maps come from the reference rig, where they were verified
-- against a live client (`PreSearing/code/OpenTyria/GmMapsConfig.c`, 2026-10-25): the tester stood the
-- character in the map, read its position back, and confirmed it landed where it should. They are data,
-- not constants, which is why they live in a table and not in the code: a new map is a row, not a
-- build.
--
-- `plane` is the third component of the reference's `Vec3f` spawn point, and it is what the client
-- calls a plane index rather than a height: 0 for every pre-Searing map measured so far.
--
-- Real, not double precision, because the client receives these as 32-bit floats and a value that
-- cannot be represented in a float is a spawn point nobody can be told about.

ALTER TABLE map_config ADD COLUMN IF NOT EXISTS spawn_x real NOT NULL DEFAULT 0;
ALTER TABLE map_config ADD COLUMN IF NOT EXISTS spawn_y real NOT NULL DEFAULT 0;
ALTER TABLE map_config ADD COLUMN IF NOT EXISTS spawn_plane smallint NOT NULL DEFAULT 0;

-- A map with no blob can never be loaded by a client, so a row claiming one is a row that hangs a
-- client at "loading". That is enforced where it can be: `asura-world` refuses to start on a map whose
-- `map_file_id` is zero, and `tools/seed-map.ps1` is the only thing that writes these rows. A CHECK
-- constraint would say the same thing here, but PostgreSQL has no `ADD CONSTRAINT IF NOT EXISTS`, and
-- a migration that cannot be re-applied safely is worse than a rule enforced one layer up (the
-- repository's tests hold every statement to that standard).

-- Record that this migration ran (crates/asura-db/src/migrations.rs; the version is the file number).
INSERT INTO schema_version (version) VALUES (5) ON CONFLICT (version) DO NOTHING;
