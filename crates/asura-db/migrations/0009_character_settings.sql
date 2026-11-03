-- The rest of the character settings blob the login reply carries.
--
-- `CharacterSettings_FromDbCharacter` (`GmChar.c`) builds a 62-byte blob for every character in the
-- `AUTH_SMSG_CHARACTER_INFO` list, and this server was sending **one captured blob for everybody**: a
-- Warrior, level 1, last seen in map 146, wearing five dyed armour pieces. It came from the reference's own
-- reply, so it looked right, and it was wrong for every character that was not that one.
--
-- What the blob is made of, and where each field comes from now:
--
--   version, last_time_played, h001E, h0021   constants (6, 0, 0, 0) -- see `GmChar.c` and `GmChar.h`
--   last_outpost                              the `map_id` this schema already keeps
--   appearance                                the packed word migration 0008 already keeps
--   last_guild_hall_id                        zero: nothing here creates guilds
--   campaign, level, is_pvp, secondary_profession, helm_status
--                                             the five columns this migration adds
--   number_of_pieces + pieces[5]              derived from the equipped bag's five armour items, exactly
--                                             as `HandleCharCreationConfirm` (`GameSrv.c:1893-1921`) does:
--                                             `file_id & 0xFFFF` and `dye_colors`, in slot order
--                                             body, legs, head, boots, gloves
--
-- The reference stores the armour pieces denormalised on the character row. This keeps deriving them from
-- the equipped bag instead: one copy of the answer, and the bag is what the client is told about anyway.
--
-- The creation defaults are the reference's own (`GameSrv.c:1880-1886`): level 1, `is_pvp` when the
-- campaign is `Campaign_None`, the helmet shown, and the unlocked-professions bit set to the primary
-- profession. `create_character` writes them; a row from before this migration reads as level 1, campaign
-- 0 (none), helmet shown, which is what those characters were being shown anyway.

ALTER TABLE characters ADD COLUMN IF NOT EXISTS level smallint NOT NULL DEFAULT 1;
ALTER TABLE characters ADD COLUMN IF NOT EXISTS campaign smallint NOT NULL DEFAULT 0;
ALTER TABLE characters ADD COLUMN IF NOT EXISTS is_pvp boolean NOT NULL DEFAULT false;
ALTER TABLE characters ADD COLUMN IF NOT EXISTS secondary_profession smallint NOT NULL DEFAULT 0;
ALTER TABLE characters ADD COLUMN IF NOT EXISTS helm_status smallint NOT NULL DEFAULT 1;

-- Record that this migration ran (crates/asura-db/src/migrations.rs; the version is the file number).
INSERT INTO schema_version (version) VALUES (9) ON CONFLICT (version) DO NOTHING;
