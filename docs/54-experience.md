# 54 - Experience, levels and skill points

**Status, 2026-11-13:** the data path is built and a test command publishes a change. The curve, the
load block's pair of numbers and the skill points a level grants are decided; the level-up moment on a
live client is not measured yet.

## 1. What the client is told, and where it comes from

`GAME_SMSG_CHARACTER_UPDATE_FACTIONS` (`0x00E9`, fifteen dwords) is the one message that carries a
character's progress. Its dwords, in order (`crates/asura-protocol/src/game/mod.rs`,
`CharacterProgress`):

| # | field | this server's source |
|---|---|---|
| 1 | total experience | `t_characters.experience` (migration 0036) |
| 2-7 | Kurzick, Luxon and Imperial current/total | zero: no faction exists in pre-Searing |
| 8-9 | two unmodelled dwords | zero |
| 10 | level | `t_characters.level` |
| 11 | morale | 100, the unmodified value a live load carries (`0x009C` says the same per agent) |
| 12-13 | Balthazar current/total | zero |
| 14-15 | skill points held / earned | `t_characters.skill_points`, `skill_points_total` |

Before this change experience was a compiled test value (500) because no column held it, and skill
points were always zero (`crates/asura-world/src/player.rs`). The load now sends the character's own
numbers. A live load for a level-13 character carries 68,113 experience and level 13
(`dump4-after`, `worldcontext.level/experience/morale`), which is the shape the block keeps.

## 2. The curve

`asura_rules::experience::Curve` holds the rule and no value. The values are
`config/World/Vanilla/experience.json`:

- level 2 costs 2,000 total experience, and each later level costs 600 more than the one before it:
  level 20 is 140,600 total (preforged `src/runtime/gw_experience_level.h`, whose comment cites the
  archived wiki `Level.html`);
- a level grants one skill point, on top of the previous level's total; level 1 has none.

The level is derived from the total, never chosen by the caller. The migration backfills an existing
row to the least experience its stored level requires, so a level-13 row does not read as level 1.

## 3. What a change sends

`crate::player::set_experience` writes the four numbers in one row update, then sends one of two
shapes:

1. **A jump of one level or less**: one `0x00EE {0, delta}`. Attribute 0 is experience; the client
   applies `+=` to the sheet (Rurik, `toolkit/authsrv/authsrv.py:5911-5960`: 23 of 23 live kill frames
   and 10 of 10 quest hand-ins). **This is what moves the experience bar.**
2. **A decrease, or a jump of more than one level**: the whole `0x00E9` block. That is the login's own
   statement and it is safe on this client; a numeric decrease has no negative delta, and a
   multi-level award is not a shape retail sends (a kill or a 500-point quest crosses at most one
   level). The bar may not animate for this shape, but the sheet and the level are written.

**Nothing else is sent.** The public level (`0x009F` 36) and the attribute points (`0x0037`) are not:
on 2026-11-13 this client crashed on a change that sent them, and the measured kill and quest paths
send neither - the client works its own level out from the award, and the server's level and points
are read at the next load.

The `0x00E9` block remains the load's own statement of the total.

The fight's copy of health and energy is not refreshed in the middle of a fight. A level-up changes
maximum health in `combat.json`'s formula, and the engine's actor spec is built when it joins the
fight; the next load or the next fight picks the new value up. Level-up effects, the level-up sound
and the "You have reached level N" line are the client's own and are not sent here.

## 4. The test command

`/.exp <total>` sets the total and lets the curve answer the level; `/xp` prints what the character
has, and `/xp <total>` sets it too. Both are ASURA test commands standing in for the quests and kills
that will award experience (preforged has the same pair, `gw_development_commands.h` and
`gw_chat_commands.h`). The command is not the feature: it is the trigger that lets the owner see the
bar move on a real client before a quest or a kill does.

`/.exp` caps at the level-20 threshold. Experience above the cap is not stored, because nothing in the
pre-Searing content awards any, and the curve gives the cap's number to level 20.

## 5. What is still open

- **A live reading of a level-up.** `/xp 2000` on a real client should move the bar by 2000 and cross to
  level 2 from the award alone. The server's level and attribute points are read at the next load; the
  client's own level-up visual is the client's. This is the reading the crash on 2026-11-13 interrupted.
- **Creature experience.** A kill still gives nothing (`docs/52-skills.md`, "Not built"). The award
  belongs to the fight's death path and needs the creature's own experience value; the wiki's per-foe
  table is recorded in `docs/36` E10 and Rurik's `killxp.py`, and is the next step.
- **Quest experience.** The quest rewards in preforged carry their own amounts (200 for Grawl
  Invasion, 250 for the profession tests, 500 for Opposition to the King); they are ported when the
  quest itself is.
- **Skill points from quests.** In preforged a quest reward can add skill points beside experience;
  this world only grants the one-per-level amount today.
