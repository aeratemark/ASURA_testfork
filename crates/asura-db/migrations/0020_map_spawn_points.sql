-- A map has a LIST of spawn points, not one.
--
-- The reference's own shape (`PreSearing/code/OpenTyria/GmMapsConfig.h`):
--
--     #define GM_MAP_CONFIG_MAX_SPAWNPOINTS 10
--     uint32_t  n_spawnpoints;
--     GmPos     spawnpoints[GM_MAP_CONFIG_MAX_SPAWNPOINTS];
--
-- and it uses the first one (`GameSrv.c:1075`: `result = srv->map_static_config->spawnpoints[0]`).
-- This table is that list: `idx` 0 is the one the world spawns a character at, the rest are kept
-- because a live capture records them and a map entered from three portals has three.
--
-- `map_config.spawn_count` becomes the count this table holds; `spawn_x`/`spawn_y`/`spawn_plane` stay
-- for the one-spawn case and are seeded into row 0 here, so nothing that reads them breaks.

CREATE TABLE IF NOT EXISTS map_spawn (
    map_id int      NOT NULL REFERENCES map_config (map_id) ON DELETE CASCADE,
    idx    smallint NOT NULL,
    x      real     NOT NULL,
    y      real     NOT NULL,
    plane  smallint NOT NULL DEFAULT 0,
    PRIMARY KEY (map_id, idx)
);

INSERT INTO map_spawn (map_id, idx, x, y, plane)
SELECT map_id, 0, spawn_x, spawn_y, spawn_plane
  FROM map_config
 WHERE spawn_count > 0
ON CONFLICT (map_id, idx) DO NOTHING;

-- Record that this migration ran (crates/asura-db/src/migrations.rs; the version is the file number).
INSERT INTO schema_version (version) VALUES (20) ON CONFLICT (version) DO NOTHING;
