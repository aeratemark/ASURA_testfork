-- The instances a character has reached: one row per (character, instance).
--
-- The owner's model, 2026-10-30 (docs/22-world-model.md): instances start locked - "all the bitmap is 0"
-- - and "as we walk and touch or reach a map, we mark it unlocked". Reaching is what unlocks; the bit
-- never gates the walk.
--
-- The client keeps the bitmap in its in-game world context: 32 bits per word, the bit for instance `n` at
-- word `n / 32` and bit `n % 32`, and it is the **fifth** of the five lists
-- `GAME_SMSG_MAP_UPDATE_UNLOCKED_LIST` (0x0094) carries. 31 words covers instances 0 to 991, which
-- contains the reference's own `MapId_Count` sentinel of 877.
--
-- Rows rather than a bitmap column, for the same reason the inventory is rows: an unlock is a fact with a
-- time, it can be queried by instance, and a bitmap in a column would have to be decoded to answer the
-- question the world map asks ("which instances has this character seen"). The column this replaces was
-- never written by anything.
--
-- Idempotent, like every migration here.

CREATE TABLE IF NOT EXISTS instance_unlock (
    character_id uuid        NOT NULL REFERENCES characters (id) ON DELETE CASCADE,
    map_id       int         NOT NULL,
    unlocked_at  timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (character_id, map_id)
);

-- Record that this migration ran (crates/asura-db/src/migrations.rs; the version is the file number).
INSERT INTO schema_version (version) VALUES (23) ON CONFLICT (version) DO NOTHING;
