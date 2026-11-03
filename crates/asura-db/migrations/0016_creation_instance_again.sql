-- The character-creation instance, back in the catalogue.
--
-- Measured from a live creation
-- (`recordings/client/feature/character_creation/character_creation-live-necro_factions.jsonl`): the
-- client asks for `AUTH_CMSG_REQUEST_GAME_INSTANCE` with `map_id 0` and `map_type 11`, and the
-- instance it enters sends no spawn point and no map blob - only `INSTANCE_LOAD_HEAD`,
-- `INSTANCE_PLAYER_DATA_START` and `CHAR_CREATION_START`. The row exists so a travel can be placed
-- on it; the instance loads no geometry.
--
-- `map_id` stays 0 because that is the id the client itself asks for.

INSERT INTO map_config (map_id, map_type, map_file_id, spawn_count, name)
VALUES (0, 11, 0, 1, 'Character Creation')
ON CONFLICT (map_id) DO NOTHING;

-- Record that this migration ran (crates/asura-db/src/migrations.rs; the version is the file number).
INSERT INTO schema_version (version) VALUES (16) ON CONFLICT (version) DO NOTHING;
