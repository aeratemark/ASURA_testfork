-- The character fields the reference stores and this server had been sending as constants.
--
-- `GameSrv_HandleCharCreationConfirm` writes a `DbCharacter` row (`GameSrv.c:1869-1886`) and the load block
-- reads it back. Four of those fields are ones this server already sends, but as constants:
--
--   sex, height, skin_color, hair_color, face_style, hair_style, race
--       -> the packed `Appearance` word `GAME_SMSG_PLAYER_CREATE` carries. `player_create` was given
--          `Appearance::REFERENCE_DEFAULT`, so every character looked like the reference's own.
--   primary_profession
--       -> `PLAYER_UPDATE_PROFESSION`. A constant (Warrior) went out for everyone.
--   unlocked_professions
--       -> `PLAYER_UPDATE_UNLOCKED_PROFESSIONS`. The constant `2` went out for everyone.
--   gold
--       -> `UPDATE_GOLD_STORAGE`. Zero went out for everyone.
--
-- This migration is the schema half of paying those four back; the four messages stop being constants in
-- the same change. The appearance word is stored packed exactly as the confirm carries it, so nothing
-- unpacks and repacks it on the way through.
--
-- What is deliberately *not* here: the rest of the reference's row -- skills, the 1024-bit unlocked-skill
-- bitmap, the four 256-bit completion bitmaps, skill points, experience, level, campaign, the per-slot
-- armour file ids and colours. Each needs a message that reads it to exist first; a column nobody loads is
-- the same liberty in a new place. They are queue item B5 in docs/10-parity-queue.md, with the reference's
-- column list recorded there.

ALTER TABLE characters ADD COLUMN IF NOT EXISTS appearance bigint NOT NULL DEFAULT 0;
ALTER TABLE characters ADD COLUMN IF NOT EXISTS primary_profession int NOT NULL DEFAULT 0;
ALTER TABLE characters ADD COLUMN IF NOT EXISTS unlocked_professions bigint NOT NULL DEFAULT 0;
ALTER TABLE characters ADD COLUMN IF NOT EXISTS gold int NOT NULL DEFAULT 0;

-- Record that this migration ran (crates/asura-db/src/migrations.rs; the version is the file number).
INSERT INTO schema_version (version) VALUES (8) ON CONFLICT (version) DO NOTHING;
