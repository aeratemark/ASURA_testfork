"""The map catalogue: which instances exist, their Gw.dat blob, spawn points and travel portals.

ASURA keeps this in PostgreSQL (``map_config`` / ``map_spawn``), seeded by the migrations in
``crates/asura-db/migrations`` and ``tools/map-config.sql``. The editor must work without a running
database, so it reads the same SQL files directly: a small, tolerant scanner replays the
``INSERT INTO map_config``, ``UPDATE map_config SET name``, ``INSERT INTO map_spawn`` and
``DELETE FROM map_spawn`` statements in file order. It is not a SQL engine; it only understands the
shapes those files use today, and anything else is ignored.

Travel portals come from ``data/portals.json`` (``tools/gen-portal-table.py``).
"""

from __future__ import annotations

import json
import re
from dataclasses import dataclass, field
from pathlib import Path

MAP_TYPES = {0: "outpost", 1: "explorable", 11: "character creation"}


@dataclass
class SpawnPoint:
    x: float
    y: float
    plane: int = 0


@dataclass
class TravelPortal:
    model_file_id: int
    x: float
    y: float
    z: float


@dataclass
class Crossing:
    """A measured doorway crossing (ASURA ``portals.rs`` ``CROSSED``)."""
    x: float
    y: float
    to_map: int | None
    arrive: tuple[float, float] | None


@dataclass
class Door:
    """A row of ASURA's ``map_portal`` table (``0025_map_portals.sql``)."""
    x: float
    y: float
    to_map: int | None
    arrive: tuple[float, float] | None
    source: str = ""


@dataclass
class MapInfo:
    map_id: int
    map_type: int = 1
    map_file_id: int = 0
    name: str = ""
    spawns: list[SpawnPoint] = field(default_factory=list)
    portals: list[TravelPortal] = field(default_factory=list)
    crossings: list[Crossing] = field(default_factory=list)
    doors: list[Door] = field(default_factory=list)

    @property
    def kind(self) -> str:
        return MAP_TYPES.get(self.map_type, "other")

    @property
    def label(self) -> str:
        return f"{self.name} ({self.map_id})" if self.name else f"map {self.map_id}"


_STR = r"'(?:[^']|'')*'"
_NUM = r"-?\d+(?:\.\d*)?(?:[eE][-+]?\d+)?"
_SPAWN = re.compile(rf"\(\s*({_NUM})\s*,\s*({_NUM})\s*,\s*({_NUM})\s*,\s*({_NUM})\s*,\s*({_NUM})\s*\)")
_STATEMENT = re.compile(r"(INSERT\s+INTO\s+map_config|INSERT\s+INTO\s+map_spawn|INSERT\s+INTO\s+map_portal|"
                        r"UPDATE\s+map_config|DELETE\s+FROM\s+map_spawn)(.*?);", re.I | re.S)
_NULLNUM = rf"(?:{_NUM}|NULL)"
_DOOR = re.compile(rf"\(\s*({_NUM})\s*,\s*({_NUM})\s*,\s*({_NUM})\s*,\s*({_NUM})\s*,\s*({_NULLNUM})\s*,"
                   rf"\s*({_NULLNUM})\s*,\s*({_NULLNUM})\s*,\s*({_STR})\s*\)", re.I)


def _sql_str(s: str) -> str:
    return s[1:-1].replace("''", "'")


def _strip_comments(sql: str) -> str:
    return "\n".join(line.split("--", 1)[0] if "'" not in line.split("--", 1)[0] or line.count("'") % 2 == 0
                     else line for line in sql.splitlines())


class Catalogue:
    def __init__(self) -> None:
        self.maps: dict[int, MapInfo] = {}

    def get(self, map_id: int) -> MapInfo:
        if map_id not in self.maps:
            self.maps[map_id] = MapInfo(map_id)
        return self.maps[map_id]

    def sorted(self) -> list[MapInfo]:
        return [self.maps[k] for k in sorted(self.maps)]

    # --- loading -----------------------------------------------------------------------------
    @classmethod
    def from_asura(cls, root: str | Path) -> "Catalogue":
        root = Path(root)
        cat = cls()
        files = sorted((root / "crates" / "asura-db" / "migrations").glob("*.sql"))
        extra = root / "tools" / "map-config.sql"
        if extra.exists():
            files.append(extra)
        for f in files:
            cat.apply_sql(f.read_text(encoding="utf-8", errors="replace"))
        portals = root / "data" / "portals.json"
        if portals.exists():
            cat.apply_portals(json.loads(portals.read_text()))
        crossed = root / "crates" / "asura-world" / "src" / "portals.rs"
        if crossed.exists():
            cat.apply_crossings_rs(crossed.read_text(encoding="utf-8"))
        return cat

    def apply_crossings_rs(self, src: str) -> None:
        pat = re.compile(r"Portal\s*\{\s*from_instance:\s*(-?\d+),\s*x:\s*([-\d.]+),\s*y:\s*([-\d.]+),\s*"
                         r"to_instance:\s*(None|Some\((-?\d+)\)),\s*arrive:\s*(None|Some\(\(([-\d.]+),\s*([-\d.]+)\)\))")
        for m in pat.finditer(src):
            info = self.get(int(m.group(1)))
            to = int(m.group(5)) if m.group(5) else None
            arrive = (float(m.group(7)), float(m.group(8))) if m.group(7) else None
            info.crossings.append(Crossing(float(m.group(2)), float(m.group(3)), to, arrive))

    def apply_sql(self, sql: str) -> None:
        sql = _strip_comments(sql)
        for verb, body in _STATEMENT.findall(sql):
            verb = " ".join(verb.upper().split())
            if verb == "INSERT INTO MAP_CONFIG":
                self._insert_config(body)
            elif verb == "INSERT INTO MAP_SPAWN":
                if "SELECT" in body.upper():
                    continue  # derived from map_config, which we already hold
                for mid, idx, x, y, plane in _SPAWN.findall(body):
                    m = self.get(int(mid))
                    i = int(idx)
                    while len(m.spawns) <= i:
                        m.spawns.append(SpawnPoint(0.0, 0.0))
                    m.spawns[i] = SpawnPoint(float(x), float(y), int(float(plane)))
            elif verb == "INSERT INTO MAP_PORTAL":
                for mid, _idx, x, y, to, ax, ay, source in _DOOR.findall(body):
                    num = lambda v: None if v.upper() == "NULL" else float(v)  # noqa: E731
                    arrive = (num(ax), num(ay)) if num(ax) is not None and num(ay) is not None else None
                    self.get(int(mid)).doors.append(
                        Door(float(x), float(y), int(float(to)) if num(to) is not None else None, arrive,
                             _sql_str(source)))
            elif verb == "UPDATE MAP_CONFIG":
                where = re.search(r"WHERE\s+map_id\s*=\s*(\d+)", body, re.I)
                if not where:
                    continue
                m = self.get(int(where.group(1)))
                name = re.search(rf"name\s*=\s*({_STR})", body, re.I)
                if name:
                    m.name = _sql_str(name.group(1))
                ft = re.search(r"map_file_id\s*=\s*(\d+)", body, re.I)
                if ft:
                    m.map_file_id = int(ft.group(1))
            elif verb == "DELETE FROM MAP_SPAWN":
                where = re.search(r"WHERE\s+map_id\s*=\s*(\d+)", body, re.I)
                if where:
                    self.get(int(where.group(1))).spawns.clear()

    def _insert_config(self, body: str) -> None:
        cols = re.search(r"\(([^)]*)\)\s*VALUES", body, re.I)
        names = [c.strip().lower() for c in cols.group(1).split(",")] if cols else []
        values = body[cols.end():] if cols else body
        values = re.split(r"ON\s+CONFLICT", values, flags=re.I)[0]
        for tup in re.findall(r"\((.*?)\)\s*(?:,|$)", values, re.S):
            fields = [f.strip() for f in re.findall(rf"\s*({_STR}|[^,]+)", tup)]
            if len(fields) != len(names):
                continue
            row = dict(zip(names, fields))
            try:
                m = self.get(int(row["map_id"]))
            except (KeyError, ValueError):
                continue
            if "map_type" in row:
                m.map_type = int(float(row["map_type"]))
            if "map_file_id" in row and row["map_file_id"].upper() != "NULL":
                m.map_file_id = int(float(row["map_file_id"]))
            if "name" in row and row["name"].startswith("'"):
                m.name = _sql_str(row["name"])
            sx, sy = row.get("spawn_x", "NULL"), row.get("spawn_y", "NULL")
            if sx.upper() != "NULL" and sy.upper() != "NULL" and not m.spawns:
                m.spawns.append(SpawnPoint(float(sx), float(sy), int(float(row.get("spawn_plane", "0")))))

    def apply_portals(self, data: dict) -> None:
        for key, items in data.items():
            m = self.get(int(key))
            m.portals = [TravelPortal(int(p.get("model_file_id", 0)), float(p["x"]), float(p["y"]),
                                      float(p.get("z", 0.0))) for p in items]

    # --- cache -------------------------------------------------------------------------------
    def to_json(self) -> dict:
        return {str(k): {"map_type": m.map_type, "map_file_id": m.map_file_id, "name": m.name,
                         "spawns": [[s.x, s.y, s.plane] for s in m.spawns],
                         "portals": [[p.model_file_id, p.x, p.y, p.z] for p in m.portals],
                         "crossings": [[c.x, c.y, c.to_map, list(c.arrive) if c.arrive else None]
                                       for c in m.crossings],
                         "doors": [[d.x, d.y, d.to_map, list(d.arrive) if d.arrive else None, d.source]
                                   for d in m.doors]}
                for k, m in sorted(self.maps.items())}

    @classmethod
    def from_json(cls, data: dict) -> "Catalogue":
        cat = cls()
        for k, v in data.items():
            m = cat.get(int(k))
            m.map_type, m.map_file_id, m.name = v["map_type"], v["map_file_id"], v["name"]
            m.spawns = [SpawnPoint(*s) for s in v.get("spawns", [])]
            m.portals = [TravelPortal(*p) for p in v.get("portals", [])]
            m.crossings = [Crossing(c[0], c[1], c[2], tuple(c[3]) if c[3] else None)
                           for c in v.get("crossings", [])]
            m.doors = [Door(d[0], d[1], d[2], tuple(d[3]) if d[3] else None, d[4]) for d in v.get("doors", [])]
        return cat
