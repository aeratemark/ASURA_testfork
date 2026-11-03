# ASURA Construction Set

A world and content editor for the [ASURA](https://github.com/apoguita/ASURA) Guild Wars server, in the
spirit of the Morrowind Construction Set with Nox's place-and-go immediacy. Written in Python with
PySide6, plus Panda3D for the 3D view.

```
pip install PySide6 panda3d numpy
python run_editor.py                # or: python -m asura_cs [profile-folder]
```

On first start, point **Edit › Preferences** at your ASURA checkout. Gw.dat and GWMB exports are
optional. Then use **File › New profile from base game…**: Ascalon City (148) opens with its spawn
points and travel portals in place.

## Scope: pre-Searing only

The editor works on pre-Searing Ascalon and nothing else: Ascalon City, Ashford Abbey, Foible's Fair,
Fort Ranik, The Barradin Estate, Lakeside County, Green Hills County, Wizard's Folly, Regent Valley,
The Northlands and The Catacombs (`asura_cs/content/scope.py`). New profiles contain only these maps,
portal destinations are picked from them, and validation, export and Apply ignore or reject anything
else. **World › Keep only pre-Searing maps** trims an older profile.

## What you get

| Area | What it does |
|---|---|
| **3D world view** (centre left) | Panda3D renders offscreen into the Qt window. Orbit, pan, zoom, WASD fly. Click to select. Drag NPCs along the terrain. Click or drop palette items onto the ground. |
| **2D view** (centre right) | Top (Nox-style), Front and Side orthographic views. Shows the hillshaded terrain, walkable path graph, navigation graph overlay, scenery props, routes and radii. Supports rubber-band select, drag-move with grid snap, and Alt-drag to rotate. |
| **Object Window** | *Place*: templates defined in Python, which you click to arm or drag into a view. *Records*: agents, dialogue, quests and scripts. *Map models*: model ids used by the map's scenery. |
| **World** | Tree of everything in the map. Group it by type, layer, group, route or agent definition, and filter it. Selection stays in sync with the views. |
| **Properties** | A form built from field metadata. Multi-select edits every selected object, and each change is one undo step. |
| **Dialogue** | TES-style editor. Nodes hold NPC text, a Lua *condition*, a Lua *result* and fallback nodes. Choices (buttons) have a kind (talk, quest available !, accept, decline, in progress ?, turn in), a quest, a target node, and Lua condition/result. **Preview** walks the conversation. |
| **Scripts** | Lua editor with highlighting, event and API snippets, and `luac -p` syntax check when Lua is installed. |
| **Quests** | Objective list (talk, fetch, kill, reach, escort, accompany, custom). Header, client fields and dialogue lines are edited in Properties. |
| **History / Validate** | `QUndoStack` history. **F7** validates references, ids, routes, spawns, client text limits, and placements off the walkable surface. |

## How it maps onto ASURA

* **Projects are ASURA profiles** (`docs/34-self-contained-profiles.md`). A profile is a self-contained
  folder: `profile.json`, `agents.json`, `dialogues.json`, `quests.json`, `maps/<id>.json` and
  `scripts/*.lua`. A new profile is a *copy* of the base content, and it keeps no link to the base. The
  editor never writes Gw.dat, the ASURA checkout or the base profile.
* **The content model follows `docs/30-world-content-and-scripting.md`**: agent definitions,
  placements, groups, routes (ordered waypoints), trigger areas, spawn points, travel portals,
  dialogue, quests and Lua scripts. Lua decides what happens and Rust executes it.
* **The base profile** is built from ASURA's own data: the `map_config`/`map_spawn` SQL in
  `crates/asura-db/migrations` and `tools/map-config.sql`, plus `data/portals.json`. Portal destinations
  and landing points come from the measured crossings in `crates/asura-world/src/portals.rs`.
* **Exports** (File › Export):
  * **ASURA SQL (spawn points)** works with ASURA today. It rewrites `map_spawn` and the `map_config`
    default spawn for every map in the profile.
  * **Content bundle (JSON)** and **Content bundle (Lua)** hold the whole profile in one file. They are
    for the loader ASURA's simulation plan describes, which doesn't exist yet. ASURA hasn't chosen a
    content format or fixed its Lua event names. The event names live in one place,
    `asura_cs/content/lua_templates.py`, so you can rename them there when it does.

### Getting spawn edits into the running server

Use **File › Apply spawn points to ASURA…** in the editor, or run `python apply_to_asura.py [profile-folder]` on its own. It:

1. reads `ASURA_DATABASE_URL` from ASURA's `.env` and finds `psql` (on PATH or under `C:\Program Files\PostgreSQL`);
2. compares the profile's spawn points with `map_config` and ticks only the maps that differ;
3. applies those maps in one transaction with `psql -v ON_ERROR_STOP=1`, the same call ASURA's supervisor makes;
4. restarts the selected world services through the running supervisor (`asura-supervisor restart world-8001`).

`asura-supervisor start` re-applies `tools/map-config.sql`, which resets the stock spawn of the catalogue
maps, including Ascalon City. After a full start, apply again. Restarting a single world doesn't re-seed.

### What ASURA can and can't show yet

ASURA currently sends one agent: the player. NPCs, dialogue and quests that you author are *content
ready for* the simulator's Phase 3 and Phase 6 (`docs/32-world-simulation-plan.md`). The editor keeps
them valid and exportable, but you won't see them in the client until ASURA can spawn agents.
Spawn points are the exception: they work end to end now.

## Real geometry

Geometry sources, all read-only:

1. **ASURA `data/path-graphs.bin`**: the walkable trapezoid mesh of every served map. No Gw.dat
   needed. It's drawn as the floor in both views and used to validate placements.
2. **Gw.dat**: the terrain heightmap and prop placements of the map blob (FFNA chunks
   `0x20000002`, `0x20000004`, `0x2000000C`). The editor uses this to put objects on the ground.
   Decoding uses ASURA's own decompressor. When `rustc` is present, the editor compiles it once from
   your checkout into the cache folder. Otherwise a pure-Python port is used, which is slow on first
   open and cached afterwards.
3. **Guild Wars Map Browser exports**: in GWMB, right-click a map and choose *Export full map*. Then add
   the folder under Preferences › GWMB exports. You get the textured terrain and the **actual prop
   meshes**, instanced at their real transforms.

## Python content definitions

Palette entries are plain Python. Put a file in `~/.asura_cs/library/` or `<profile>/library/`:

```python
from asura_cs.library import agent_template, object_template

agent_template("Ascalon/Guards", "Ascalon Guard",
               agent=dict(id="ascalon_guard", name="Ascalon Guard", allegiance="ally", level=15,
                          profession="warrior", behaviour="guard", model_file_id=0),
               placement=dict(movement="stand"))
```

`examples/library/observed_npcs.py` contains an NPC with client fields that were observed to render.

### Client fields (thanks to Rurik's research)

A GW client only draws an agent whose definition it has been told about. The **Client** group on an
agent definition holds the fields that the [Rurik](https://github.com/GameJawnsInc/Rurik) research
project documents for this: model file id, composite model id, **name string ids** (the nameplate is a
list of GW string ids rather than text), appearance word, flags word and speed. Quests carry the
client quest id, name and region string ids, log flags, and the offer, in-progress and turn-in lines.
Dialogue choice kinds map to the option icons Rurik measured. The validator warns about Rurik's
measured text limits: 122 UTF-16 units per dialog line, and 128 for option labels and quest log fields.
Every agent definition and quest has **Source** and **Evidence** fields, because ASURA asks for
evidence behind base-game values. Nothing from Rurik is a dependency. Only the facts are used,
restated with credit, as ASURA's ADR 0011 prescribes.

## Layout

```
asura_cs/
  formats/   path_graphs (ASURAPG1), catalogue (ASURA SQL), gwdat + fa_decompress (+ native helper),
             ffna_map (terrain/props), gwmb_export
  content/   model, store, base_profile, commands (undo), validate, lua_templates
  library/   palette templates (Python)
  export/    asura_sql, content_json, lua_bundle
  world/     renderer-independent 3D math and meshes
  ui/        main window, document (QUndoStack), 2D/3D viewports, palette, hierarchy, properties,
             dialogue / script / quest editors
tests/       pytest: formats, content, undo, export, 3D math, native-vs-Python decompressor,
             and a UI wiring smoke test against a fake Qt
```

Run `pytest`. The tests look for ASURA at `$ASURA_ROOT`.

## Credits

* **Guild Wars Map Browser** by Jonathan Bjørn Greve, https://github.com/Jonathan-Greve/GuildWarsMapBrowser.
  The FFNA map chunk layouts (terrain, props, bounds, filename hashing) and the GWMB export format
  follow its source, used under its license, which requires this credit and link.
* **ASURA**: the Gw.dat index and decompressor (ported, and compiled from your checkout for the fast
  path), the path-graph dump format, and the map catalogue.
* **Rurik** (MIT): client field notes for NPC definitions, dialog option kinds and text limits.

No ArenaNet data ships with this tool.
