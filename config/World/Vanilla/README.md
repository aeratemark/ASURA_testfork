# The Vanilla pack

The faithful base game: the maps, spawn points, doorways and later NPCs, quests and scripts that
reproduce the live server ASURA is measured against.

What is here today: `character_spawn.json` (who spawns where, read once by the world at start-up) and
`character_defaults.json` (what a new character gets) and `party_sizes.json` (how many players a party may
hold on each map), all read through `asura-world`; and `maps/`, which
is what the instance table is built from. The rest of the content is still in the database and moves here
in the order `docs/38-layer-decisions.md` D22 records: write the package, switch the readers, drop the
table last. `docs/37-rework-layers.md` layer 13 tracks what has moved.

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
