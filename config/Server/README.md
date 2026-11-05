# Server settings

How this machine runs ASURA, as opposed to what the world contains.

Six files today:

- `config/services.toml`, one level up: which services start, their arguments, ports, health checks
  and log paths. It is the supervisor's own file.
- `config/Server/defaults.json`: the values a world falls back on when a row says nothing. One
  documented key today - `default_outpost`, the outpost a character enters when its row names neither a
  map nor a last outpost - and every key carries its own `doc` sentence, because JSON has no comments
  and a file of bare numbers cannot be reviewed. No key has a default in code: a missing file stops
  the socket layer at start-up (`docs/38-layer-decisions.md`, D34).
- `config/Server/intervals.json`: the intervals every program runs on - the world tick, the ping
  interval and value, the login path's probe timeout, and the freshness window the login path and
  the socket layer share. Read by `asura-config`, once per process; an environment variable still
  overrides a single value for an experiment (`docs/38-layer-decisions.md`, D33).
- `config/Server/movement.json`: the tolerances a client's movement reports are judged by. One key
  today - `max_allowed_correction`, how far a client may place itself when it cancels a walk and
  still be believed. The reference's own value is 100 and is the fallback in code; this machine runs
  it at 150. Read by `asura-config`, once per process, same precedence as the intervals file.
- `config/Server/spawns.json`: how a character is placed around the spawn point it entered at. One
  key today - `spawn_scatter`, the radius in map units of the circle a character lands in around a
  random point of the map's own spawn list (250 here; zero stacks everyone on the point). Read by
  `asura-config`, once per process, same precedence as the movement file.

- `config/Server/titles.json`: the title definitions - per title, the values its track message
  (`0x00F6`) and rank records (`0x00F3`) carry that do not change with play: scope, flags, the coded
  strings, every rung with the points that reach it, and the rank a player may show it from. An
  `enabled` switch turns the whole feature off. Coded strings are hex UTF-16 units, at most 7. Read by
  `asura-config` once per process (`ASURA_TITLES_CONFIG` names another file); a file that breaks one
  of the client's crash rules is refused whole and titles are off, with the reason on stderr
  (`docs/40-titles-system-research.md`). Experimental, and here rather than under `config/World/` on
  the owner's instruction (2026-11-05). Adding a title is adding an entry, copied from a capture of a
  character that has the title.

What does not belong here: anything that describes the game. A map, a spawn point, a doorway or a
quest is world content, and lives in a package under `config/World/`
(`docs/38-layer-decisions.md`, D23, D24).
