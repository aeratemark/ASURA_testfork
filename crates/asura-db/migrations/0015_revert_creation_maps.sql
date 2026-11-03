-- Remove the two maps the character-creation work added, so the catalogue is the reference's own
-- twelve again.
--
-- The creation instance (`map_id 0`, type 11, no blob) and the Factions outpost (`map_id 212`) were
-- added on 2026-10-28 by migrations 0013 and 0014. That work is being reverted: the creation feature
-- moved a character into the instance and left it there, and the world refuses to host a map with no
-- `map_file_id` (crates/asura-world/src/main.rs), so the rows have to go with the code.
--
-- Measured values kept for whoever picks the work up: the Factions outpost is map 212, blob 156969,
-- spawn (-2132, 1054) plane 0; live's creation instance is map 416 and sends no blob at all
-- (`recordings/client/feature/character_creation/character_creation-live-necro_factions.jsonl`).

DELETE FROM map_config WHERE map_id IN (0, 212);

-- Record that this migration ran (crates/asura-db/src/migrations.rs; the version is the file number).
INSERT INTO schema_version (version) VALUES (15) ON CONFLICT (version) DO NOTHING;
