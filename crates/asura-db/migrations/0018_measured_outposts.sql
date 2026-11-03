-- The maps a live tour of outposts measured, with the names the client uses.
--
-- Two sources, both recorded:
--   * the numbers - map id, blob id, spawn - from
--     recordings/client/feature/character_creation/maps-tour.jsonl: one 0x019A (map id) and the
--     0x0196 that follows (blob, x, y), per load;
--   * the names from the client's own map tables,
--     Py4GW_Stealth/py4gw/enums_src/map_enums.py (outposts, 277 entries), matched by id.
--
-- Every row here is an outpost (type 0), because the tour visited outposts. Explorables carry type 1
-- and none was captured, so none is claimed: a wrong type is a loading screen that never ends.
--
-- What these maps do not have is a path graph. The dump in data/path-graphs.bin holds the reference's
-- 12 maps, so click-to-walk in these outposts has no server-side path until the dump is regenerated
-- with --dump-path-graphs, which needs a local Gw.dat.

INSERT INTO map_config (map_id, map_type, map_file_id, spawn_count, spawn_x, spawn_y, spawn_plane, name)
VALUES  (479, 0, 210612, 1, 25164.219, 6372.237, 0, 'Champions Dawn'),
  (857, 0, 338456, 1, 247.713, -2538.752, 0, 'Embark Beach'),
  (248, 0, 165811, 1, -5474, -4873.943, 0, 'Great Temple of Balthazar'),
  (281, 0, 165826, 1, -2030, 4700, 0, 'Zaishen Challenge outpost'),
  (282, 0, 165842, 1, -2034, 1034, 0, 'Zaishen Elite outpost'),
  (188, 0, 165083, 1, 2229, 1691.014, 0, 'Random Arenas outpost'),
  (796, 0, 246766, 1, 965.882, 3222.825, 0, 'Codex Arena outpost'),
  (330, 0, 165788, 1, 3978, -4905, 0, 'Heroes Ascent outpost'),
  (795, 0, 350586, 1, -7207.918, -4645.031, 0, 'Zaishen Menagerie outpost'),
  (5, 0, 127496, 1, 5362, 7779, 0, 'Guild Hall - Hunter''s Isle'),
  (818, 0, 352944, 1, -9167.137, 13288.662, 0, 'Kamadan Jewel of Istan - Halloween'),
  (544, 0, 249289, 1, 5115.209, -5100.12, 0, 'Chahbek Village outpost'),
  (493, 0, 214476, 1, -2728, 16217, 0, 'Consulate Docks outpost'),
  (431, 0, 210513, 1, -4384.741, 5231.431, 0, 'Sunspear Great Hall'),
  (502, 0, 210626, 1, -940.495, 922.409, 0, 'The Astralarium'),
  (491, 0, 210623, 1, 2446.191, -495.813, 0, 'Jokanur Diggings outpost'),
  (492, 0, 210629, 1, 1813, 391, 0, 'Blacktide Den outpost'),
  (487, 0, 210727, 1, -20136, 12599, 0, 'Beknur Harbor'),
  (489, 0, 210732, 1, -1714, -3430, 0, 'Kodlonu Hamlet'),
  (477, 0, 212775, 1, -18614, -10017, 0, 'Nundu Bay outpost'),
  (381, 0, 213190, 1, -1885.302, 2394.212, 0, 'Yohlon Haven'),
  (554, 0, 212792, 1, -3838, -1971, 0, 'Dajkah Inlet outpost'),
  (387, 0, 216120, 1, -2227.781, -1317.764, 0, 'Sunspear Sanctuary'),
  (426, 0, 213215, 1, 1881, 490, 0, 'Pogahn Passage outpost'),
  (424, 0, 212586, 1, -1993.495, 2059.409, 0, 'Kodonur Crossroads outpost'),
  (425, 0, 214069, 1, -15950, 10325, 0, 'Rilohn Refuge outpost'),
  (427, 0, 215315, 1, -11345, -13338, 0, 'Moddok Crevice outpost'),
  (378, 0, 212979, 1, -22, -735, 0, 'Wehhan Terraces'),
  (393, 0, 211038, 1, -12205.781, 361.766, 0, 'Chantry of Secrets'),
  (421, 0, 212844, 1, 24398, 15021, 0, 'Venta Cemetery outpost'),
  (478, 0, 214076, 1, 3128, 706, 0, 'Gate of Desolation outpost'),
  (545, 0, 210316, 1, -944.118, -1845.175, 0, 'Remains of Sahlahja outpost'),
  (438, 0, 210198, 1, -14493, 6391, 0, 'Bone Palace'),
  (480, 0, 210324, 1, -3132.918, 14441.969, 0, 'Ruins of Morah outpost'),
  (440, 0, 210292, 1, 225.209, -4167.12, 0, 'The Mouth of Torment'),
  (396, 0, 211075, 1, 2727, 671, 0, 'Mihanu Township'),
  (433, 0, 212428, 1, 2755, 3393, 0, 'Dzagonur Bastion outpost'),
  (403, 0, 212420, 1, -16349, 17888, 0, 'Honur Hill'),
  (376, 0, 212976, 1, -18618, 17940, 0, 'Camp Hojanu'),
  (808, 0, 157839, 1, 3988.858, 9433.65, 0, 'Lions Arch - Halloween'),
  (117, 0, 52839, 1, 7686, 11065, 0, 'Thirsty River outpost'),
  (116, 0, 52838, 1, 11940.081, 9255.969, 0, 'Dunes of Despair outpost'),
  (152, 0, 52835, 1, -16473, -18541, 0, 'Heroes Audience'),
  (154, 0, 52166, 1, -15182, 18797, 0, 'Destinys Gorge'),
  (38, 0, 52837, 1, -16006.302, 592.787, 0, 'Augury Rock outpost'),
  (109, 0, 52918, 1, 3012.219, -591.765, 0, 'The Amnoon Oasis'),
  (118, 0, 52840, 1, 15084, 6816, 0, 'Elona Reach outpost'),
  (120, 0, 52842, 1, -17890, 17395, 0, 'The Dragon''s Lair outpost'),
  (814, 0, 352813, 1, 1267.209, 1149.88, 0, 'Tomb of the Primeval Kings - Halloween'),
  (29, 0, 13906, 1, 6757.219, -28600.766, 0, 'Fort Ranik outpost'),
  (81, 0, 22052, 1, 4760.078, 7034.975, 0, 'Ascalon City'),
  (642, 0, 288299, 1, -925.781, 3312.235, 0, 'Eye of the North outpost')
ON CONFLICT (map_id) DO NOTHING;

-- Record that this migration ran (crates/asura-db/src/migrations.rs; the version is the file number).
INSERT INTO schema_version (version) VALUES (18) ON CONFLICT (version) DO NOTHING;
