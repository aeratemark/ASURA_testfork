-- A character's experience, and the skill points its levels have granted (docs/54-experience.md).
--
-- WHY COLUMNS ON THE CHARACTER. `characters.level` already exists and the load block already carries
-- experience: the fifteen-dword `GAME_SMSG_CHARACTER_UPDATE_FACTIONS` puts total experience first and
-- level tenth (`crates/asura-protocol/src/game/mod.rs`, `CharacterProgress`). Experience had no column,
-- so the load sent a test value. This is the state that value describes.
--
-- WHAT THE BACKFILL IS. An existing row keeps its level, and gets the least experience that level
-- requires, so a level-13 character does not read as level 1. The curve is the pack's at the time of
-- this migration (`config/World/Vanilla/experience.json`; preforged `gw_experience_level.h`: level L+1
-- costs 2000 + 600 * (L - 1), cumulative). The skill points are one per level above the first.
--
-- OPERATIONAL STATE, NOT CONTENT (`docs/38-layer-decisions.md`, D21). Idempotent, like every migration.

ALTER TABLE t_characters ADD COLUMN IF NOT EXISTS experience int NOT NULL DEFAULT 0
    CHECK (experience >= 0);
ALTER TABLE t_characters ADD COLUMN IF NOT EXISTS skill_points int NOT NULL DEFAULT 0
    CHECK (skill_points >= 0);
ALTER TABLE t_characters ADD COLUMN IF NOT EXISTS skill_points_total int NOT NULL DEFAULT 0
    CHECK (skill_points_total >= skill_points);

UPDATE t_characters
   SET experience = (level - 1) * 2000 + 600 * (level - 1) * (level - 2) / 2,
       skill_points = level - 1,
       skill_points_total = level - 1
 WHERE level > 1
   AND experience = 0
   AND skill_points = 0
   AND skill_points_total = 0;

INSERT INTO schema_version (version) VALUES (36) ON CONFLICT (version) DO NOTHING;
