"""Profile folders on disk.

::

    <profile>/
      profile.json          id, name, description, based_on, format
      agents.json           agent definitions
      dialogues.json        dialogues with nodes and choices
      quests.json           quests with objectives
      maps/<map_id>.json    map header + objects (typed), routes, groups
      scripts/<id>.lua      Lua sources

JSON is used because it diffs well and ASURA's docs name it as the likely exchange format; nothing in
ASURA reads this layout yet (see ``asura_cs.export`` for what it can consume today). Writes go to a
temporary file then replace, so an interrupted save never leaves half a file.
"""

from __future__ import annotations

import json
import os
from dataclasses import fields
from pathlib import Path

from .model import (OBJECT_TYPES, AgentDef, Dialogue, DialogueChoice, DialogueNode, Group, MapContent,
                    Objective, Profile, Quest, Route, Script)

FORMAT = "asura-cs-profile/1"


def _record_to_dict(rec) -> dict:
    out = {}
    for f in fields(rec):
        v = getattr(rec, f.name)
        if f.name in ("nodes", "objectives", "source"):
            continue
        out[f.name] = list(v) if isinstance(v, list) else v
    return out


def _fill(cls, data: dict):
    names = {f.name for f in fields(cls)}
    obj = cls()
    for k, v in data.items():
        if k in names:
            setattr(obj, k, v)
    return obj


def dialogue_to_dict(d: Dialogue) -> dict:
    out = _record_to_dict(d)
    out["nodes"] = [{"id": n.id, "text": n.text, "condition": n.condition, "result": n.result,
                     "alternatives": list(n.alternatives),
                     "choices": [vars(c).copy() for c in n.choices]} for n in d.nodes]
    return out


def dialogue_from_dict(data: dict) -> Dialogue:
    d = _fill(Dialogue, {k: v for k, v in data.items() if k != "nodes"})
    d.nodes = []
    for n in data.get("nodes", []):
        node = _fill(DialogueNode, {k: v for k, v in n.items() if k != "choices"})
        node.choices = [_fill(DialogueChoice, c) for c in n.get("choices", [])]
        node.alternatives = list(n.get("alternatives", []))
        d.nodes.append(node)
    return d


def quest_to_dict(q: Quest) -> dict:
    out = _record_to_dict(q)
    out["objectives"] = [vars(o).copy() for o in q.objectives]
    return out


def quest_from_dict(data: dict) -> Quest:
    q = _fill(Quest, {k: v for k, v in data.items() if k != "objectives"})
    q.objectives = [_fill(Objective, o) for o in data.get("objectives", [])]
    return q


def map_to_dict(m: MapContent) -> dict:
    return {
        "map_id": m.map_id, "name": m.name, "map_type": m.map_type, "map_file_id": m.map_file_id,
        "objects": [{"type": o.TYPE, **_record_to_dict(o)} for o in m.objects],
        "routes": [_record_to_dict(r) for r in m.routes],
        "groups": [_record_to_dict(g) for g in m.groups],
    }


def map_from_dict(data: dict) -> MapContent:
    m = MapContent(int(data["map_id"]), data.get("name", ""), int(data.get("map_type", 1)),
                   int(data.get("map_file_id", 0)))
    for o in data.get("objects", []):
        cls = OBJECT_TYPES.get(o.get("type"))
        if cls is None:
            continue  # unknown type from a newer editor: dropped, reported by load()
        m.objects.append(_fill(cls, {k: v for k, v in o.items() if k != "type"}))
    m.routes = [_fill(Route, r) for r in data.get("routes", [])]
    m.groups = [_fill(Group, g) for g in data.get("groups", [])]
    return m


def _write_json(path: Path, data) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(path.suffix + ".tmp")
    tmp.write_text(json.dumps(data, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    os.replace(tmp, path)


def save(profile: Profile, folder: str | Path) -> None:
    folder = Path(folder)
    folder.mkdir(parents=True, exist_ok=True)
    _write_json(folder / "profile.json", {"format": FORMAT, "id": profile.id, "name": profile.name,
                                          "description": profile.description, "based_on": profile.based_on})
    _write_json(folder / "agents.json", [_record_to_dict(a) for a in profile.agents])
    _write_json(folder / "dialogues.json", [dialogue_to_dict(d) for d in profile.dialogues])
    _write_json(folder / "quests.json", [quest_to_dict(q) for q in profile.quests])
    maps_dir = folder / "maps"
    maps_dir.mkdir(exist_ok=True)
    keep = set()
    for mid, m in sorted(profile.maps.items()):
        p = maps_dir / f"{mid}.json"
        _write_json(p, map_to_dict(m))
        keep.add(p.name)
    for stale in maps_dir.glob("*.json"):   # a map removed from the profile is removed from disk
        if stale.name not in keep:
            stale.unlink()
    scripts_dir = folder / "scripts"
    scripts_dir.mkdir(exist_ok=True)
    keep = set()
    for s in profile.scripts:
        p = scripts_dir / f"{s.id}.lua"
        tmp = p.with_suffix(".lua.tmp")
        tmp.write_text(s.source, encoding="utf-8")
        os.replace(tmp, p)
        keep.add(p.name)
    for stale in scripts_dir.glob("*.lua"):
        if stale.name not in keep:
            stale.unlink()


def load(folder: str | Path) -> Profile:
    folder = Path(folder)
    head = json.loads((folder / "profile.json").read_text(encoding="utf-8"))
    if head.get("format") != FORMAT:
        raise ValueError(f"{folder}: unsupported profile format {head.get('format')!r}")
    p = Profile(head.get("id", folder.name), head.get("name", ""), head.get("description", ""),
                head.get("based_on", ""))

    def read(name):
        f = folder / name
        return json.loads(f.read_text(encoding="utf-8")) if f.exists() else []

    p.agents = [_fill(AgentDef, a) for a in read("agents.json")]
    p.dialogues = [dialogue_from_dict(d) for d in read("dialogues.json")]
    p.quests = [quest_from_dict(q) for q in read("quests.json")]
    for f in sorted((folder / "maps").glob("*.json")):
        m = map_from_dict(json.loads(f.read_text(encoding="utf-8")))
        p.maps[m.map_id] = m
    for f in sorted((folder / "scripts").glob("*.lua")):
        p.scripts.append(Script(id=f.stem, source=f.read_text(encoding="utf-8")))
    return p


def is_profile(folder: str | Path) -> bool:
    return (Path(folder) / "profile.json").exists()
