-- A map's name, so `/maps` can answer the way the reference does (M5.1).
--
-- `map_config` has identified a map by its id, its blob and its spawn since 0001. What it has never had
-- is a name, so this server's `/maps` could only ever print numbers, while the reference prints names
-- from a table compiled into it (`GmCommands.c`, `s_teleport_targets`: "Lakeside County", "Ascalon City
-- (pre-Searing)", ...). A command whose answer is a list of integers is a command nobody uses.
--
-- The names seeded for the development maps are the client's own, read off the same reference table --
-- data, not constants, which is why they live in a row rather than in the code. A map with no name is a
-- map that prints its id; that is a worse answer, not a broken one.

ALTER TABLE map_config ADD COLUMN IF NOT EXISTS name text NOT NULL DEFAULT '';

-- Record that this migration ran (crates/asura-db/src/migrations.rs; the version is the file number).
INSERT INTO schema_version (version) VALUES (7) ON CONFLICT (version) DO NOTHING;
