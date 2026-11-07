# World packages

One folder per package, and one package loaded per server start
(`docs/38-layer-decisions.md`, D23, D24).

```
config/World/Vanilla     the faithful base game: the pack ASURA reproduces
config/World/<name>      any other pack, copied from Vanilla and edited
```

A pack is self-contained: it does not inherit from `Vanilla`, and loading it does not reach into
another folder for missing content. A pack that has three quests has three quests.

Nothing reads this folder yet. Content still lives in the database, and it leaves in the order
D22 describes: write the package, switch its readers, drop the table last. `Vanilla` is where the
first content table (map settings) lands when that move is scheduled.

The packaging format inside a pack is chosen (`docs/38-layer-decisions.md`, D38). The maps live under
`maps/`, and one map is reached by three names before its id:

```
config/World/Vanilla/maps/<campaign>/<area>/<subarea>/<map id>/basic.json
```

- `<campaign>` and `<area>` are the client's own `AreaInfo.campaign` and `AreaInfo.region` for that map;
  `<subarea>` is its kind, and there are four of them, spelled exactly like this: `Arenas`,
  `Explorables`, `Missions`, `Outposts`. A dungeon level counts as `Explorables` - the client instances
  it the same way.
- `<map id>` is the map's id, and the folder holds that map's own files: `basic.json` (id, name,
  `map_type`, `map_file_id`, `instance_value`, and `name_id`: the client's string id of the map's name,
  which a party countdown's window needs - a map without it gets no window; `tools/add-map-name-ids.py`
  writes it from `Gw.exe`), `spawns.json` (the spawn list, and the only place a spawn
  is stated) and `portals.json` (the map's arches, and the landings it gives to arrivals).
- The instance table this server sends is built by walking this tree, so a folder's position is what
  places a map; the walk is recursive and a map with no folder is a map the server does not know.
- A map whose place is not known sits in `maps/unplaced/<map id>/`, with the same files.
