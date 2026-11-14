# The Vanilla pack

The faithful base game: the maps, spawn points, doorways and later NPCs, quests and scripts that
reproduce the live server ASURA is measured against.

What is here today: `character_spawn.json` (who spawns where, read once by the world at start-up) and
`character_defaults.json` (what a new character gets), `chat.json` (the bounds `/roll` and a whisper
decide by), `party_sizes.json` (how many players a party may
hold on each map) `items.json` (the static item definitions an owned `t_item` row may name), and `skills.json`,
`combat.json` and `attributes.json` (the pre-Searing skills, the numbers a fight runs on and the attribute
rules, `docs/52-skills.md`), `experience.json` (the experience curve and the skill points a level grants,
`docs/54-experience.md`), `dialogs.json` (the greeting and choices an NPC says, `docs/55-npc-dialogs.md`),
`quests.json` (each quest's log text, details, markers and captured variants, `docs/58-quests.md`),
`creatures.json` (how a hostile creature notices, chases, gives up and
respawns, `docs/52-skills.md` "Mobs"), `vendors.json` (the merchants, collectors and Xunlai storage NPCs a talk opens, and the item rows
their windows offer, `docs/53-npc-services.md`), all read through `asura-world`; `npcs/` (the NPC types and the compass range) with one `npcs.json` per map folder
that has NPCs (`docs/49-npc-framework.md`); and `maps/`, which is what the instance table is built from. The rest of the content
is still in the database and moves here in the order `docs/38-layer-decisions.md` D22 records: write the
package, switch the readers, drop the table last. `docs/37-rework-layers.md` layer 13 tracks what has
moved. The item definition/instance boundary is argued in `docs/47-item-system.md`.

`maps/` is laid out as `<campaign>/<area>/<subarea>/<map id>/` (`docs/38-layer-decisions.md`, D38), and
every map id holds its own `basic.json`:

- 137 ids: 51 under `Core`, 39 `Prophecies`, 22 `Factions`, 11 `Nightfall`, 5 `Eye of the North`.
- 9 sit in `maps/unplaced/`, because no name table and no page answered which campaign and area they
  belong to. They keep their folder and their instance value, so the table a client receives is complete;
  what they lack is a place, and a folder nobody measured is a guess with a path on it.
- The walk that builds the table goes as deep as the tree does, so a map found four levels down is a map
  like any other (`crates/asura-world/src/instance_table.rs`).

Editing this pack is editing the base game. A custom pack is a copy of this folder under another
name in `config/World/`, never an edit here.
