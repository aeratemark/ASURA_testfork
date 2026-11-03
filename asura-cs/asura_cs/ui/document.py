"""The open profile, its undo stack, the current map and the selection: one object every panel shares."""

from __future__ import annotations

import itertools
from pathlib import Path

from PySide6.QtCore import QObject, Signal
from PySide6.QtGui import QUndoCommand, QUndoStack

from ..content import commands as C
from ..content import store
from ..content.model import MapContent, MapObject, Profile
from ..gamedata import GameData, MapScene

_merge_ids = itertools.count(1000)
_MERGE_INT: dict = {}


class QtCommand(QUndoCommand):
    """Adapter: a core :class:`Command` driven by ``QUndoStack``."""

    def __init__(self, cmd: C.Command, hub: C.ChangeHub):
        super().__init__(cmd.text)
        self.cmd, self.hub = cmd, hub

    def redo(self):
        self.cmd.redo(self.hub)

    def undo(self):
        self.cmd.undo(self.hub)

    def id(self) -> int:  # noqa: A003 - Qt API name
        key = self.cmd.merge_key
        if key is None:
            return -1
        if key not in _MERGE_INT:
            _MERGE_INT[key] = next(_merge_ids)
        return _MERGE_INT[key]

    def mergeWith(self, other) -> bool:  # noqa: N802 - Qt API name
        return isinstance(other, QtCommand) and self.cmd.merge(other.cmd)


class EditorDocument(QObject):
    profileReset = Signal()                 # a different profile was opened
    mapChanged = Signal(int)                # current map id
    contentChanged = Signal(str, object)    # kind ('added'|'removed'|'changed'), record
    selectionChanged = Signal(list)         # list of selected records (map objects or records)
    armedChanged = Signal(object)           # palette template armed for click-placing (or None)
    message = Signal(str)                   # status/log line

    def __init__(self, gamedata: GameData):
        super().__init__()
        self.gamedata = gamedata
        self.profile = Profile()
        self.folder: Path | None = None
        self.hub = C.ChangeHub()
        self.hub.subscribe(self._on_hub)
        self.undo_stack = QUndoStack(self)
        self.map_id: int | None = None
        self.selection: list = []
        self.armed = None
        self._drag_counter = itertools.count()

    # -- profile ------------------------------------------------------------------------------
    def set_profile(self, profile: Profile, folder: Path | None) -> None:
        self.profile, self.folder = profile, folder
        self.undo_stack.clear()
        self.selection = []
        self.profileReset.emit()
        from ..content.scope import in_scope
        first = next((m for m in sorted(profile.maps) if in_scope(m)), None)
        prefer = 148 if 148 in profile.maps else first
        self.set_map(prefer)
        self.undo_stack.setClean()

    def save(self, folder: Path | None = None) -> Path:
        folder = Path(folder or self.folder)
        store.save(self.profile, folder)
        self.folder = folder
        self.undo_stack.setClean()
        self.message.emit(f"Saved profile to {folder}")
        return folder

    @property
    def dirty(self) -> bool:
        return not self.undo_stack.isClean()

    # -- map ----------------------------------------------------------------------------------
    @property
    def map(self) -> MapContent | None:
        return self.profile.maps.get(self.map_id) if self.map_id is not None else None

    def scene(self) -> MapScene | None:
        m = self.map
        return self.gamedata.scene(m.map_id, m.map_file_id) if m else None

    def set_map(self, map_id: int | None) -> None:
        if map_id == self.map_id and map_id is not None:
            return
        self.map_id = map_id
        self.select([])
        if map_id is not None:
            sc = self.scene()
            for n in sc.notes if sc else []:
                self.message.emit(f"{self.map.label()}: {n}")
        self.mapChanged.emit(map_id if map_id is not None else -1)

    def ensure_map(self, map_id: int) -> MapContent:
        """Add a catalogue map to the profile if it is not there yet (undoable)."""
        if map_id in self.profile.maps:
            return self.profile.maps[map_id]
        from ..content.base_profile import map_from_catalogue
        info = self.gamedata.catalogue.get(map_id)
        m = map_from_catalogue(info)
        maps = self.profile.maps

        class _AddMap(C.Command):
            text = f"Add map {m.label()}"

            def redo(self, hub):
                maps[m.map_id] = m
                hub.emit("added", m, maps)

            def undo(self, hub):
                maps.pop(m.map_id, None)
                hub.emit("removed", m, maps)

        self.push(_AddMap())
        return m

    # -- edits --------------------------------------------------------------------------------
    def push(self, cmd: C.Command) -> None:
        self.undo_stack.push(QtCommand(cmd, self.hub))

    def new_drag_key(self):
        return ("drag", next(self._drag_counter))

    def _on_hub(self, kind, obj, info):
        if kind == "removed" and obj in self.selection:
            self.selection = [s for s in self.selection if s is not obj]
            self.selectionChanged.emit(list(self.selection))
        self.contentChanged.emit(kind, obj)

    def ground_z(self, x: float, y: float, fallback: float = 0.0) -> float:
        sc = self.scene()
        z = sc.ground_z(x, y) if sc else None
        return fallback if z is None else z

    def place_template(self, template, x: float, y: float, z: float | None = None) -> list:
        if self.map is None:
            return []
        if z is None:
            z = self.ground_z(x, y)
        adds = template.instantiate(self.profile, self.map_id, x, y, z)
        self.push(C.AddRecords(adds, f"Place {template.name}"))
        placed = [r for _, r in adds if isinstance(r, MapObject)]
        self.select(placed)
        return placed

    def place_agent_def(self, agent_def, x: float, y: float, z: float | None = None):
        from ..content.model import AgentPlacement
        if self.map is None:
            return None
        z = self.ground_z(x, y) if z is None else z
        p = AgentPlacement(id=self.profile.unique_id(f"{agent_def.id}_ref"), name=agent_def.name,
                           agent=agent_def.id, x=x, y=y, z=z)
        self.push(C.AddRecords([(self.map.objects, p)], f"Place {agent_def.name}"))
        self.select([p])
        return p

    def selected_objects(self) -> list[MapObject]:
        return [s for s in self.selection if isinstance(s, MapObject)]

    def delete_selected(self) -> None:
        m = self.map
        items = []
        for o in self.selected_objects():
            if m and o in m.objects:
                items.append((m.objects, o))
        if items:
            self.push(C.RemoveRecords(items, f"Delete {len(items)} object(s)"))
            self.select([])

    def duplicate_selected(self) -> None:
        objs = self.selected_objects()
        if not objs or self.map is None:
            return
        dups = C.duplicate(self.profile, objs)
        self.push(C.AddRecords([(self.map.objects, d) for d in dups], f"Duplicate {len(dups)}"))
        self.select(dups)

    def drop_to_ground(self) -> None:
        changes = [(o, {"z": self.ground_z(o.x, o.y, o.z)}) for o in self.selected_objects()]
        if changes:
            self.push(C.SetFields(changes, "Drop to ground"))

    def rotate_selected(self, degrees: float) -> None:
        changes = [(o, {"yaw": (o.yaw + degrees) % 360.0}) for o in self.selected_objects()]
        if changes:
            self.push(C.SetFields(changes, "Rotate"))

    def set_fields(self, objs: list, values: dict, text: str = "Change") -> None:
        if objs:
            self.push(C.SetFields([(o, dict(values)) for o in objs], text))

    def rename(self, rec, new: str) -> str | None:
        """Change an editor id and every reference to it, as one undo step. Returns an error or None."""
        from ..content.model import valid_id
        p = self.profile
        old = rec.id
        if new == old:
            return None
        if not valid_id(new) or new in p.all_ids():
            return f"{new!r} is not a valid, unused editor id."
        changes = [(rec, {"id": new})]
        ref_fields = {"agent": ("agent", "giver"), "dialogue": ("dialogue",), "script": ("script",),
                      "route": ("route",), "group": ("group_id",)}.get(rec.TYPE, ())
        colls = [p.agents, p.quests] + [m.objects for m in p.maps.values()]
        for coll in colls:
            for o in coll:
                for fname in ref_fields:
                    if o is not rec and getattr(o, fname, None) == old:
                        changes.append((o, {fname: new}))
        if rec.TYPE == "quest":
            for q in p.quests:
                if old in q.prerequisites:
                    changes.append((q, {"prerequisites": [new if x == old else x for x in q.prerequisites]}))
        self.push(C.SetFields(changes, f"Rename {old} -> {new}"))
        return None

    # -- selection / placing ----------------------------------------------------------------
    def select(self, objs: list, add: bool = False) -> None:
        if add:
            new = list(self.selection)
            for o in objs:
                if any(o is s for s in new):
                    new = [s for s in new if s is not o]  # toggle
                else:
                    new.append(o)
            objs = new
        if [id(o) for o in objs] == [id(s) for s in self.selection]:
            return
        self.selection = list(objs)
        self.selectionChanged.emit(list(self.selection))

    def arm(self, template) -> None:
        self.armed = template
        self.armedChanged.emit(template)
