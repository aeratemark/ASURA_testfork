-- The outpost a Factions character starts in, measured from a live creation.
--
-- From `recordings/client/feature/character_creation/character_creation-live-necro_factions.jsonl`:
-- after the confirm, live redirects the client to map 212 (`0x01A6` carries 212, `0x0099` repeats
-- it, and the `0x019A` load info that follows names 212), and the spawn point live sends for that
-- load is blob 156969 at (-2132.0, 1054.0) on plane 0.
--
-- The id and the numbers are copied, not chosen. `map_type` is 0 because the client entered it as an
-- outpost; nothing in the capture contradicts that.
--
-- The other campaigns have no measurement yet, so there is no row for them: a guessed outpost would
-- put a number on the wire that live never sends.

INSERT INTO map_config (map_id, map_type, map_file_id, spawn_count, spawn_x, spawn_y, spawn_plane, name)
VALUES (212, 0, 156969, 1, -2132.0, 1054.0, 0, 'Factions starting outpost')
ON CONFLICT (map_id) DO NOTHING;

-- Record that this migration ran (crates/asura-db/src/migrations.rs; the version is the file number).
INSERT INTO schema_version (version) VALUES (14) ON CONFLICT (version) DO NOTHING;
