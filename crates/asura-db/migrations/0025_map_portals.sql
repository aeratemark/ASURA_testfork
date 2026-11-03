-- Travel portals as content (docs/32 Phase 1): one row per doorway, read by asura-world at start-up.
--
-- Seeded from the two tables compiled into crates/asura-world/src/portals.rs:
--   * every measured crossing (CROSSED), verbatim;
--   * every portal prop (PORTALS); a prop within 600 units of a measured crossing in the same
--     instance carries that crossing's destination and landing point, so the gate fires from either
--     spot (the Ascalon City gate is the prop at (7378, 5429) and the crossing at (7401, 5543)).
-- Plus Lakeside County <-> Regent Valley, which no recording has crossed: destinations from the
-- Guild Wars Wiki's exit lists, landing points chosen on the path graph, 500+ units inside the map
-- and clear of every doorway. `source` says which rows are which.
--
-- ON CONFLICT DO NOTHING: re-applying the schema never overwrites an edited row.

CREATE TABLE IF NOT EXISTS map_portal (
    map_id   int     NOT NULL,
    idx      int     NOT NULL,
    x        real    NOT NULL,
    y        real    NOT NULL,
    to_map   int     NULL,
    arrive_x real    NULL,
    arrive_y real    NULL,
    source   text    NOT NULL DEFAULT '',
    PRIMARY KEY (map_id, idx),
    CHECK ((arrive_x IS NULL) = (arrive_y IS NULL))
);

INSERT INTO map_portal (map_id, idx, x, y, to_map, arrive_x, arrive_y, source) VALUES
    (145, 0, 14442.0, 2135.0, 164, -13151.0, -7080.0, 'measured crossing'),
    (145, 1, 16823.0, -10892.0, NULL, NULL, NULL, 'portal prop'),
    (145, 2, 19411.0, 2990.0, NULL, NULL, NULL, 'portal prop'),
    (145, 3, 5961.0, -22656.0, NULL, NULL, NULL, 'portal prop'),
    (146, 0, 7198.0, 5177.0, 148, 7739.0, 5900.0, 'measured crossing'),
    (146, 1, -5499.0, 13795.0, 147, -11574.0, -16170.0, 'measured crossing'),
    (146, 2, -14124.0, 10037.0, 160, 21932.0, 12966.0, 'measured crossing'),
    (146, 3, -10962.0, -6225.0, 164, -11672.0, -6209.0, 'measured crossing'),
    (146, 4, -13460.0, -19979.0, 161, 9449.0, 19702.0, 'measured crossing'),
    (146, 5, 7378.0, 5429.0, 148, 7739.0, 5900.0, 'portal prop + measured crossing 310 u away'),
    (146, 6, 4384.0, -19844.0, 162, -16636.0, 16896.0, 'derived: wiki exits, landing on the path graph'),
    (146, 7, -13897.0, -20108.0, 161, 9449.0, 19702.0, 'portal prop + measured crossing 456 u away'),
    (146, 8, -5488.0, 13494.0, 147, -11574.0, -16170.0, 'portal prop + measured crossing 301 u away'),
    (146, 9, -14540.0, 10054.0, 160, 21932.0, 12966.0, 'portal prop + measured crossing 416 u away'),
    (146, 10, -11309.0, -6226.0, 164, -11672.0, -6209.0, 'portal prop + measured crossing 347 u away'),
    (147, 0, -11635.0, -16776.0, 146, -5434.0, 11799.0, 'measured crossing'),
    (147, 1, 26113.0, 4767.0, NULL, NULL, NULL, 'portal prop'),
    (147, 2, -11632.0, -17226.0, 146, -5434.0, 11799.0, 'portal prop + measured crossing 450 u away'),
    (147, 3, -20684.0, -20666.0, NULL, NULL, NULL, 'portal prop'),
    (148, 0, 7401.0, 5543.0, 146, 6602.0, 4485.0, 'measured crossing'),
    (148, 1, 7378.0, 5429.0, 146, 6602.0, 4485.0, 'portal prop + measured crossing 116 u away'),
    (148, 2, 4384.0, -19844.0, NULL, NULL, NULL, 'portal prop'),
    (148, 3, -13897.0, -20108.0, NULL, NULL, NULL, 'portal prop'),
    (148, 4, -5488.0, 13494.0, NULL, NULL, NULL, 'portal prop'),
    (148, 5, -14540.0, 10054.0, NULL, NULL, NULL, 'portal prop'),
    (148, 6, -11309.0, -6226.0, NULL, NULL, NULL, 'portal prop'),
    (160, 0, 21932.0, 12966.0, 146, -13891.0, 10107.0, 'measured crossing'),
    (160, 1, -7315.0, 1468.0, 163, -6940.0, 1430.0, 'measured crossing'),
    (160, 2, -6894.0, -16820.0, 161, -19103.0, 19512.0, 'measured crossing'),
    (161, 0, 10288.0, 19795.0, 146, -12914.0, -20062.0, 'measured crossing'),
    (161, 1, 462.0, 7495.0, 165, 159.0, 8456.0, 'measured crossing'),
    (161, 2, -19064.0, 19796.0, 160, -6742.0, -16419.0, 'measured crossing'),
    (162, 0, 22623.0, 6993.0, 166, 22798.0, 10205.0, 'measured crossing'),
    (162, 1, -17120.0, 17020.0, 146, 4429.0, -19346.0, 'derived: wiki exits, landing on the path graph'),
    (162, 2, -26040.0, -13185.0, NULL, NULL, NULL, 'portal prop'),
    (162, 3, 22595.0, 7257.0, 166, 22798.0, 10205.0, 'portal prop + measured crossing 265 u away'),
    (163, 0, -7442.0, 1442.0, 160, -7459.0, 1458.0, 'measured crossing'),
    (163, 1, -7339.0, 1458.0, 160, -7459.0, 1458.0, 'measured crossing'),
    (163, 2, 22967.0, -17036.0, NULL, NULL, NULL, 'portal prop'),
    (163, 3, 22324.0, 13126.0, NULL, NULL, NULL, 'portal prop'),
    (163, 4, 25555.0, -3154.0, NULL, NULL, NULL, 'portal prop'),
    (163, 5, 6822.0, -17963.0, NULL, NULL, NULL, 'portal prop'),
    (163, 6, -6793.0, -16845.0, NULL, NULL, NULL, 'portal prop'),
    (163, 7, -3250.0, 9209.0, NULL, NULL, NULL, 'portal prop'),
    (163, 8, -7361.0, 1435.0, 160, -7459.0, 1458.0, 'portal prop + measured crossing 32 u away'),
    (164, 0, -11595.0, -6336.0, 146, -10853.0, -6221.0, 'measured crossing'),
    (164, 1, -13754.0, -7076.0, 145, 14442.0, 2135.0, 'measured crossing'),
    (164, 2, -11716.0, -6244.0, 146, -10853.0, -6221.0, 'measured crossing'),
    (164, 3, 7378.0, 5429.0, NULL, NULL, NULL, 'portal prop'),
    (164, 4, 4384.0, -19844.0, NULL, NULL, NULL, 'portal prop'),
    (164, 5, -13897.0, -20108.0, NULL, NULL, NULL, 'portal prop'),
    (164, 6, -5488.0, 13494.0, NULL, NULL, NULL, 'portal prop'),
    (164, 7, -14540.0, 10054.0, NULL, NULL, NULL, 'portal prop'),
    (164, 8, -11309.0, -6226.0, 146, -10853.0, -6221.0, 'portal prop + measured crossing 306 u away'),
    (165, 0, 223.0, 8069.0, 161, 633.0, 7270.0, 'measured crossing'),
    (165, 1, 10679.0, 19828.0, NULL, NULL, NULL, 'portal prop'),
    (165, 2, 20040.0, -10113.0, NULL, NULL, NULL, 'portal prop'),
    (165, 3, -5466.0, 18901.0, NULL, NULL, NULL, 'portal prop'),
    (165, 4, 426.0, 7688.0, 161, 633.0, 7270.0, 'portal prop + measured crossing 432 u away'),
    (165, 5, -19081.0, 20019.0, NULL, NULL, NULL, 'portal prop'),
    (166, 0, 22550.0, 7410.0, 162, 22614.0, 6849.0, 'measured crossing'),
    (166, 1, -17120.0, 17020.0, NULL, NULL, NULL, 'portal prop'),
    (166, 2, -26040.0, -13185.0, NULL, NULL, NULL, 'portal prop'),
    (166, 3, 22595.0, 7257.0, 162, 22614.0, 6849.0, 'portal prop + measured crossing 159 u away'),
    (449, 0, -6751.0, 19576.0, NULL, NULL, NULL, 'portal prop'),
    (449, 1, -6026.0, 16727.0, NULL, NULL, NULL, 'portal prop'),
    (449, 2, -9263.0, 16821.0, NULL, NULL, NULL, 'portal prop'),
    (449, 3, -3440.0, 16726.0, NULL, NULL, NULL, 'portal prop'),
    (449, 4, -7396.0, 17058.0, NULL, NULL, NULL, 'portal prop'),
    (449, 5, -5928.0, 14931.0, NULL, NULL, NULL, 'portal prop'),
    (449, 6, -7297.0, 6184.0, NULL, NULL, NULL, 'portal prop')
ON CONFLICT (map_id, idx) DO NOTHING;

-- Record that this migration ran (crates/asura-db/src/migrations.rs; the version is the file number).
INSERT INTO schema_version (version) VALUES (25) ON CONFLICT (version) DO NOTHING;
