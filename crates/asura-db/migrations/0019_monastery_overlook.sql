-- Map 212 has the name the client uses for it.
--
-- The row was added from the live capture as "Factions starting outpost", which was honest when the
-- name was unknown. The client's own map tables name 212 "Monastery Overlook"
-- (Py4GW_Stealth/py4gw/enums_src/map_enums.py, outposts), and that is the name a character standing
-- there sees on its loading screen.

UPDATE map_config SET name = 'Monastery Overlook' WHERE map_id = 212;

-- Record that this migration ran (crates/asura-db/src/migrations.rs; the version is the file number).
INSERT INTO schema_version (version) VALUES (19) ON CONFLICT (version) DO NOTHING;
