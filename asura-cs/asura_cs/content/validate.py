"""Profile validation (docs/34: "a missing required reference is a validation error").

Returns a list of :class:`Issue`. Errors block export; warnings are shown in the log dock.
Optional geometric checks use the path graph: an agent or spawn placed off the walkable surface is
reported, because the server will not be able to path it.
"""

from __future__ import annotations

import re
import shutil
import subprocess
import tempfile
from dataclasses import dataclass
from pathlib import Path

from .scope import SCOPE_NAME, in_scope
from .model import (CHOICE_WIRE_KIND, DIALOG_LINE_UNITS, FIELD_UNITS, AgentPlacement, Area, Profile, SpawnPoint,
                    TravelPortal, Waypoint, valid_id)


def utf16_units(s: str) -> int:
    return len(s.encode("utf-16-le")) // 2


@dataclass
class Issue:
    level: str          # "error" | "warning"
    where: str          # human-readable location
    message: str
    obj: object = None  # the record, so the UI can select it


def validate(profile: Profile, path_graphs=None, check_lua: bool = True) -> list[Issue]:
    issues: list[Issue] = []
    E = lambda w, m, o=None: issues.append(Issue("error", w, m, o))  # noqa: E731
    W = lambda w, m, o=None: issues.append(Issue("warning", w, m, o))  # noqa: E731

    seen: dict[str, str] = {}
    qids: dict[int, str] = {}
    for q in profile.quests:
        if q.quest_id and q.quest_id in qids:
            E(f"quest {q.id}", f"client quest id {q.quest_id} is also used by {qids[q.quest_id]}", q)
        qids.setdefault(q.quest_id, q.id)

    def claim(rid: str, where: str, obj):
        if not valid_id(rid):
            E(where, f"invalid editor id {rid!r} (letters, digits, _; not starting with a digit)", obj)
        elif rid in seen:
            E(where, f"duplicate editor id {rid!r} (also used by {seen[rid]})", obj)
        else:
            seen[rid] = where

    agents = {a.id for a in profile.agents}
    dialogues = {d.id for d in profile.dialogues}
    quests = {q.id for q in profile.quests}
    scripts = {s.id for s in profile.scripts}

    for a in profile.agents:
        w = f"agent {a.id}"
        claim(a.id, w, a)
        if a.dialogue and a.dialogue not in dialogues:
            E(w, f"dialogue {a.dialogue!r} does not exist", a)
        if a.script and a.script not in scripts:
            E(w, f"script {a.script!r} does not exist", a)
        if not a.model_file_id:
            W(w, "no model file id: the client has nothing to draw", a)
        if not a.enc_name:
            W(w, "no name string ids: the client shows no nameplate (names are string ids, not text)", a)

    for d in profile.dialogues:
        w = f"dialogue {d.id}"
        claim(d.id, w, d)
        node_ids = [n.id for n in d.nodes]
        if len(set(node_ids)) != len(node_ids):
            E(w, "two nodes share an id", d)
        if d.nodes and d.start not in node_ids:
            E(w, f"start node {d.start!r} does not exist", d)
        for n in d.nodes:
            for alt in n.alternatives:
                if alt not in node_ids:
                    E(w, f"node {n.id}: alternative {alt!r} does not exist", d)
            longest = max((utf16_units(line) for line in n.text.splitlines()), default=0)
            if longest > DIALOG_LINE_UNITS:
                W(w, f"node {n.id}: a line is {longest} UTF-16 units; one dialog text message holds "
                     f"{DIALOG_LINE_UNITS} - split it into shorter lines", d)
            for c in n.choices:
                if c.goto and c.goto != "end" and c.goto not in node_ids:
                    E(w, f"node {n.id}: choice {c.text!r} goes to missing node {c.goto!r}", d)
                if utf16_units(c.text) > FIELD_UNITS:
                    W(w, f"node {n.id}: choice label longer than {FIELD_UNITS} units", d)
                if c.kind in CHOICE_WIRE_KIND and c.quest not in quests:
                    E(w, f"node {n.id}: {c.kind} choice {c.text!r} needs an existing quest (got {c.quest!r})", d)
            if not n.choices:
                W(w, f"node {n.id} has no choices; the player can only close it", d)

    for q in profile.quests:
        w = f"quest {q.id}"
        claim(q.id, w, q)
        if q.giver and q.giver not in agents:
            E(w, f"giver {q.giver!r} is not an agent definition", q)
        for pre in q.prerequisites:
            if pre not in quests:
                E(w, f"prerequisite {pre!r} does not exist", q)
        if q.script and q.script not in scripts:
            E(w, f"script {q.script!r} does not exist", q)
        if not q.objectives:
            W(w, "no objectives", q)
        for fname in ("description", "objectives_text"):
            if utf16_units(getattr(q, fname)) > FIELD_UNITS:
                W(w, f"{fname} is longer than the {FIELD_UNITS}-unit wire field", q)
        if not q.quest_id:
            W(w, "no client quest id: the client cannot show it in the quest log", q)
        for fname in ("offer_dialogue", "in_progress_dialogue", "objective_dialogue", "turn_in_dialogue"):
            if utf16_units(getattr(q, fname)) > DIALOG_LINE_UNITS:
                W(w, f"{fname} is longer than one {DIALOG_LINE_UNITS}-unit dialog line", q)

    for s in profile.scripts:
        claim(s.id, f"script {s.id}", s)

    for mid, m in sorted(profile.maps.items()):
        if not in_scope(mid):
            E(m.label(), f"outside {SCOPE_NAME}: remove it (World > Keep only {SCOPE_NAME} maps)")
            continue
        routes = {r.id for r in m.routes}
        groups = {g.id for g in m.groups}
        graph = path_graphs.for_map(mid, m.map_file_id) if path_graphs else None
        for r in m.routes:
            claim(r.id, f"{m.label()} route {r.id}", r)
            if len(m.waypoints(r.id)) < 2:
                W(f"{m.label()} route {r.id}", "a route needs at least two waypoints", r)
        for g in m.groups:
            claim(g.id, f"{m.label()} group {g.id}", g)
        idx = [s.index for s in m.spawns()]
        if idx and sorted(idx) != list(range(len(idx))):
            E(m.label(), f"spawn indices must be 0..{len(idx) - 1} without gaps, found {sorted(idx)}")
        for o in m.objects:
            w = f"{m.label()} {o.TYPE_LABEL.lower()} {o.id}"
            claim(o.id, w, o)
            if isinstance(o, AgentPlacement):
                if not o.agent:
                    E(w, "no agent definition", o)
                elif o.agent not in agents:
                    E(w, f"agent definition {o.agent!r} does not exist", o)
                if o.group_id and o.group_id not in groups:
                    E(w, f"group {o.group_id!r} does not exist in this map", o)
                if o.movement == "route" and o.route not in routes:
                    E(w, f"movement is 'route' but route {o.route!r} does not exist", o)
                if o.movement == "roam" and o.roam_radius <= 0:
                    W(w, "movement is 'roam' with a zero radius", o)
                if o.script and o.script not in scripts:
                    E(w, f"script {o.script!r} does not exist", o)
            if isinstance(o, Waypoint) and o.route not in routes:
                E(w, f"route {o.route!r} does not exist", o)
            if isinstance(o, Area) and o.script and o.script not in scripts:
                E(w, f"script {o.script!r} does not exist", o)
            if isinstance(o, TravelPortal) and o.to_map and not in_scope(o.to_map):
                E(w, f"destination map {o.to_map} is outside {SCOPE_NAME}", o)
            elif isinstance(o, TravelPortal) and o.to_map and o.to_map not in profile.maps:
                W(w, f"destination map {o.to_map} is not in this profile", o)
            if graph is not None and isinstance(o, (AgentPlacement, SpawnPoint, Waypoint)):
                if graph.find_trapezoid(o.x, o.y) is None:
                    W(w, "stands outside the walkable path graph", o)

    if check_lua:
        issues.extend(check_lua_sources(profile))
    return issues


# ----------------------------------------------------------------------------------------------
# Lua


def find_luac() -> str | None:
    for name in ("luac5.4", "luac54", "luac", "luac5.3"):
        p = shutil.which(name)
        if p:
            return p
    return None


def lua_syntax_error(source: str, chunk_kind: str = "chunk") -> str | None:
    """``luac -p`` if available; ``None`` when OK or when no luac is installed.

    chunk_kind: "chunk" (a script file), "expr" (a condition), "stmts" (a result box).
    """
    luac = find_luac()
    if not luac or not source.strip():
        return None
    if chunk_kind == "expr":
        source = f"return ({source})"
    with tempfile.NamedTemporaryFile("w", suffix=".lua", delete=False, encoding="utf-8") as f:
        f.write(source)
        name = f.name
    try:
        r = subprocess.run([luac, "-p", name], capture_output=True, text=True, timeout=10)
        if r.returncode != 0:
            return re.sub(r"^.*?:(\d+):", r"line \1:", (r.stderr or r.stdout).strip().splitlines()[-1])
        return None
    finally:
        Path(name).unlink(missing_ok=True)


def check_lua_sources(profile: Profile) -> list[Issue]:
    if not find_luac():
        return []
    out = []
    for s in profile.scripts:
        err = lua_syntax_error(s.source)
        if err:
            out.append(Issue("error", f"script {s.id}", err, s))
    for d in profile.dialogues:
        for n in d.nodes:
            for label, src, kind in [("condition", n.condition, "expr"), ("result", n.result, "stmts")] + \
                    [(f"choice '{c.text}' condition", c.condition, "expr") for c in n.choices] + \
                    [(f"choice '{c.text}' result", c.result, "stmts") for c in n.choices]:
                err = lua_syntax_error(src, kind)
                if err:
                    out.append(Issue("error", f"dialogue {d.id} node {n.id} {label}", err, d))
    for m in profile.maps.values():
        for o in m.objects:
            for fname, kind in (("spawn_condition", "expr"), ("on_enter", "stmts"), ("on_leave", "stmts")):
                src = getattr(o, fname, "")
                err = lua_syntax_error(src, kind) if src else None
                if err:
                    out.append(Issue("error", f"{m.label()} {o.id} {fname}", err, o))
    return out
