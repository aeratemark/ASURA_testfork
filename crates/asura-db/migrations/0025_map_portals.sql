-- Travel portals as content (docs/32 Phase 1): one row per doorway, read by asura-world at start-up.
--
-- Every pre-Searing travel portal, from the game's own map data (Gw.dat), with a destination and a
-- landing point for each, so all of pre-Searing Ascalon can be walked through.
--
-- Where the rows come from (asura-cs/tools/dump_portals.py, then the pairing below):
--   * Positions: every prop of the portal model (Gw.dat file 0xA825 = 43045, and its twin mesh
--     0x858B = 34187) in the six pre-Searing map blobs. Those are 113021 (Lakeside, Ascalon City,
--     Ashford Abbey), 116016 (Catacombs), 116025 (Northlands), 113190 (Green Hills, Barradin),
--     113355 (Wizard's Folly, Foible's Fair) and 113437 (Regent Valley, Fort Ranik). The server's
--     path graphs know these same maps under other blob ids (3806xx/3807xx); the coordinates agree.
--   * Destinations: a doorway between two maps is placed in both maps' blobs with the same height,
--     facing and scale, offset by a whole-tile shift. That shift is the same for every shared portal,
--     so the six blobs sit in one world frame (offsets from Lakeside: Green Hills +36864,+3072;
--     Northlands -6144,-30720; Wizard's Folly +24576,+39936; Regent Valley -21504,+36864; Catacombs
--     +30720,+9216, and 3700 lower). Each portal joins the maps whose blobs share it, checked against
--     the Guild Wars Wiki exit lists and against where the path graph is walkable.
--   * Landing points: the measured ones (live crossings, portals.rs CROSSED) wherever a crossing was
--     recorded; otherwise a walkable point 550-850 units inside the destination, clear of its
--     doorways (the same method lands within ~300-600 units of the measured points). Every landing
--     point is at least ~360 units from every arch of its map, outside portals::REACH (150).
--   * Only the arches trigger. The recorded crossing points are not doorways: they are where a
--     character stood once the live server had already changed the map, 300-450 units past the arch,
--     and one of them sat on the Catacombs landing point. Row numbers keep their old values, so the
--     gaps are where those rows were.
--
-- Rows marked 'derived' have not been walked yet: the first live crossing of each confirms or moves it.
-- The Catacombs blob also carries copies of the surface portals above it; only its own exits are listed.
-- Map 449 keeps its portal props without destinations (not pre-Searing).
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

-- A database seeded by an earlier version of this file also holds the recorded crossing points as
-- doorways; they fired before the arch and sent arriving characters straight back. Remove them.
DELETE FROM map_portal WHERE source = 'measured crossing';
-- ...and the Barradin -> Green Hills landing point, which was 101 units from the arch, unless edited since.
UPDATE map_portal SET arrive_x = -8198, arrive_y = 1583
 WHERE map_id = 163 AND to_map = 160 AND arrive_x = -7459 AND arrive_y = 1458;

INSERT INTO map_portal (map_id, idx, x, y, to_map, arrive_x, arrive_y, source) VALUES
    (146, 0, 7378.0, 5429.0, 148, 7739.0, 5900.0, 'portal prop (Gw.dat), destination measured'),  -- Lakeside County -> Ascalon City
    (146, 2, 4384.0, -19844.0, 162, -16636.0, 16896.0, 'portal prop (Gw.dat), derived in 0025 (wiki exits + path graph)'),  -- Lakeside County -> Regent Valley
    (146, 3, -13897.0, -20108.0, 161, 9449.0, 19702.0, 'portal prop (Gw.dat), destination measured'),  -- Lakeside County -> Wizard's Folly
    (146, 5, -5488.0, 13494.0, 147, -11574.0, -16170.0, 'portal prop (Gw.dat), destination measured'),  -- Lakeside County -> The Northlands
    (146, 7, -14540.0, 10054.0, 160, 21932.0, 12966.0, 'portal prop (Gw.dat), destination measured'),  -- Lakeside County -> Green Hills County
    (146, 9, -11309.0, -6226.0, 164, -11672.0, -6209.0, 'portal prop (Gw.dat), destination measured'),  -- Lakeside County -> Ashford Abbey
    (148, 0, 7378.0, 5429.0, 146, 6602.0, 4485.0, 'portal prop (Gw.dat), destination measured'),  -- Ascalon City -> Lakeside County
    (164, 0, -11309.0, -6226.0, 146, -10853.0, -6221.0, 'portal prop (Gw.dat), destination measured'),  -- Ashford Abbey -> Lakeside County
    (164, 3, -14157.0, -7083.0, 145, 14442.0, 2135.0, 'portal prop (Gw.dat), destination measured'),  -- Ashford Abbey -> The Catacombs
    (164, 5, -15986.0, -7065.0, 145, 14442.0, 2135.0, 'portal prop (Gw.dat), second Catacombs arch, destination as its twin'),  -- Ashford Abbey -> The Catacombs
    (147, 0, -11632.0, -17226.0, 146, -5434.0, 11799.0, 'portal prop (Gw.dat), destination measured'),  -- The Northlands -> Lakeside County
    (147, 2, 26113.0, 4767.0, NULL, NULL, NULL, 'portal prop (Gw.dat), no known exit (Northlands only leads to Lakeside)'),  -- The Northlands (no exit)
    (160, 0, 22324.0, 13126.0, 146, -13891.0, 10107.0, 'portal prop (Gw.dat), destination measured'),  -- Green Hills County -> Lakeside County
    (160, 2, -7361.0, 1435.0, 163, -6940.0, 1430.0, 'portal prop (Gw.dat), destination measured'),  -- Green Hills County -> The Barradin Estate
    (160, 4, -6793.0, -16845.0, 161, -19103.0, 19512.0, 'portal prop (Gw.dat), destination measured'),  -- Green Hills County -> Wizard's Folly
    (160, 6, -3250.0, 9209.0, 145, -10946.0, 16271.0, 'portal prop (Gw.dat), derived: world-frame pairing + path graph'),  -- Green Hills County -> The Catacombs
    (163, 0, -7361.0, 1435.0, 160, -8198.0, 1583.0, 'portal prop (Gw.dat), destination measured, landing point moved clear of the arch (path graph)'),  -- The Barradin Estate -> Green Hills County
    (161, 0, 10679.0, 19828.0, 146, -12914.0, -20062.0, 'portal prop (Gw.dat), destination measured'),  -- Wizard's Folly -> Lakeside County
    (161, 2, -19081.0, 20019.0, 160, -6742.0, -16419.0, 'portal prop (Gw.dat), destination measured'),  -- Wizard's Folly -> Green Hills County
    (161, 4, 426.0, 7688.0, 165, 159.0, 8456.0, 'portal prop (Gw.dat), destination measured'),  -- Wizard's Folly -> Foible's Fair
    (161, 6, 20040.0, -10113.0, 162, -25203.0, -13037.0, 'portal prop (Gw.dat), derived: world-frame pairing + path graph'),  -- Wizard's Folly -> Regent Valley
    (161, 7, -5466.0, 18901.0, 145, 5536.0, -21920.0, 'portal prop (Gw.dat), derived: world-frame pairing + path graph'),  -- Wizard's Folly -> The Catacombs
    (165, 0, 426.0, 7688.0, 161, 633.0, 7270.0, 'portal prop (Gw.dat), destination measured'),  -- Foible's Fair -> Wizard's Folly
    (162, 0, -17120.0, 17020.0, 146, 4429.0, -19346.0, 'portal prop (Gw.dat), derived in 0025 (wiki exits + path graph)'),  -- Regent Valley -> Lakeside County
    (162, 1, -26040.0, -13185.0, 161, 19304.0, -10538.0, 'portal prop (Gw.dat), derived: world-frame pairing + path graph'),  -- Regent Valley -> Wizard's Folly
    (162, 2, 22595.0, 7257.0, 166, 22798.0, 10205.0, 'portal prop (Gw.dat), destination measured'),  -- Regent Valley -> Fort Ranik
    (166, 0, 22595.0, 7257.0, 162, 22614.0, 6849.0, 'portal prop (Gw.dat), destination measured'),  -- Fort Ranik -> Regent Valley
    (145, 0, 14904.0, 2095.0, 164, -13151.0, -7080.0, 'portal prop (Gw.dat), destination measured'),  -- The Catacombs -> Ashford Abbey
    (145, 2, 16563.0, 2133.0, 164, -13151.0, -7080.0, 'portal prop (Gw.dat), second Ashford arch, destination as its twin'),  -- The Catacombs -> Ashford Abbey
    (145, 3, -10295.0, 15725.0, 160, -4049.0, 9500.0, 'portal prop (Gw.dat), derived: world-frame pairing + path graph'),  -- The Catacombs -> Green Hills County
    (145, 4, 5961.0, -22656.0, 161, -5757.0, 18102.0, 'portal prop (Gw.dat), derived: world-frame pairing + path graph'),  -- The Catacombs -> Wizard's Folly
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
