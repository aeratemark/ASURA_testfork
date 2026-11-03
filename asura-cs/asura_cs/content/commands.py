"""Undoable edits, independent of Qt.

Each command mutates the profile and notifies a :class:`ChangeHub`; the UI listens to the hub and
refreshes. ``asura_cs.ui.undo`` wraps these in ``QUndoCommand`` so the editor gets a real
``QUndoStack`` (history view, clean state, Ctrl+Z/Ctrl+Y), while tests drive them directly.

Commands hold references to the record objects themselves; records are identity-compared
(``eq=False``), so a moved NPC stays the same object through undo/redo.
"""

from __future__ import annotations

import copy
from typing import Any, Callable


class ChangeHub:
    """Tiny observer: ``hub.subscribe(fn)``; fn(kind, obj, info) with kind in
    {'added', 'removed', 'changed', 'reset'}."""

    def __init__(self) -> None:
        self._subs: list[Callable] = []
        self.muted = 0

    def subscribe(self, fn: Callable) -> None:
        self._subs.append(fn)

    def unsubscribe(self, fn: Callable) -> None:
        if fn in self._subs:
            self._subs.remove(fn)

    def emit(self, kind: str, obj: Any = None, info: Any = None) -> None:
        if self.muted:
            return
        for fn in list(self._subs):
            fn(kind, obj, info)


class Command:
    text = "Edit"
    merge_key: Any = None  # commands with equal non-None keys may merge (continuous drags)

    def redo(self, hub: ChangeHub) -> None:
        raise NotImplementedError

    def undo(self, hub: ChangeHub) -> None:
        raise NotImplementedError

    def merge(self, other: "Command") -> bool:
        return False


class AddRecords(Command):
    def __init__(self, items: list[tuple[list, Any]], text: str = "Add"):
        self.items = items  # (container list, record)
        self.text = text

    def redo(self, hub):
        for coll, rec in self.items:
            coll.append(rec)
            hub.emit("added", rec, coll)

    def undo(self, hub):
        for coll, rec in reversed(self.items):
            coll.remove(rec)
            hub.emit("removed", rec, coll)


class RemoveRecords(Command):
    def __init__(self, items: list[tuple[list, Any]], text: str = "Delete"):
        self.items = items
        self.text = text
        self._positions: list[int] = []

    def redo(self, hub):
        self._positions = []
        for coll, rec in self.items:
            i = coll.index(rec)
            self._positions.append(i)
            del coll[i]
            hub.emit("removed", rec, coll)

    def undo(self, hub):
        for (coll, rec), i in reversed(list(zip(self.items, self._positions))):
            coll.insert(i, rec)
            hub.emit("added", rec, coll)


class SetFields(Command):
    """Set attributes on one or more objects. ``changes``: list of (obj, {field: new_value})."""

    def __init__(self, changes: list[tuple[Any, dict]], text: str = "Change", merge_key: Any = None):
        self.changes = changes
        self.old = [(obj, {k: copy.deepcopy(getattr(obj, k)) for k in vals}) for obj, vals in changes]
        self.text = text
        self.merge_key = merge_key

    def _apply(self, pairs, hub):
        for obj, vals in pairs:
            for k, v in vals.items():
                setattr(obj, k, copy.deepcopy(v))
            hub.emit("changed", obj, tuple(vals))

    def redo(self, hub):
        self._apply(self.changes, hub)

    def undo(self, hub):
        self._apply(self.old, hub)

    def merge(self, other):
        if self.merge_key is None or other.merge_key != self.merge_key:
            return False
        if [id(o) for o, _ in self.changes] != [id(o) for o, _ in other.changes]:
            return False
        self.changes = [(o, {**a, **b}) for (o, a), (_, b) in zip(self.changes, other.changes)]
        return True


class Mutate(Command):
    """Arbitrary in-place change of one object, captured by snapshot (dialogue nodes, objectives...).

    ``fn(obj)`` performs the edit; undo restores a deep copy of the object's ``__dict__``.
    """

    def __init__(self, obj: Any, fn: Callable[[Any], None], text: str = "Edit"):
        self.obj, self.fn, self.text = obj, fn, text
        self.before = copy.deepcopy(obj.__dict__)
        self.after: dict | None = None

    def redo(self, hub):
        if self.after is None:
            self.fn(self.obj)
            self.after = copy.deepcopy(self.obj.__dict__)
        else:
            self.obj.__dict__.update(copy.deepcopy(self.after))
        hub.emit("changed", self.obj, None)

    def undo(self, hub):
        self.obj.__dict__.clear()
        self.obj.__dict__.update(copy.deepcopy(self.before))
        hub.emit("changed", self.obj, None)


class Macro(Command):
    def __init__(self, children: list[Command], text: str):
        self.children, self.text = children, text

    def redo(self, hub):
        for c in self.children:
            c.redo(hub)

    def undo(self, hub):
        for c in reversed(self.children):
            c.undo(hub)


class SimpleStack:
    """A Qt-free undo stack with the same semantics QUndoStack gives the UI (used by tests/CLI)."""

    def __init__(self, hub: ChangeHub):
        self.hub = hub
        self.done: list[Command] = []
        self.undone: list[Command] = []

    def push(self, cmd: Command) -> None:
        cmd.redo(self.hub)
        self.undone.clear()
        if self.done and self.done[-1].merge(cmd):
            return
        self.done.append(cmd)

    def undo(self) -> None:
        if self.done:
            c = self.done.pop()
            c.undo(self.hub)
            self.undone.append(c)

    def redo(self) -> None:
        if self.undone:
            c = self.undone.pop()
            c.redo(self.hub)
            self.done.append(c)


def duplicate(profile, objs: list, offset: tuple[float, float] = (100.0, 100.0)) -> list:
    """Copies of map objects with fresh ids, offset on the ground plane (not yet added)."""
    out, issued = [], set()
    for o in objs:
        c = copy.deepcopy(o)
        c.id = profile.unique_id(o.id, issued)
        issued.add(c.id)
        c.x += offset[0]
        c.y += offset[1]
        out.append(c)
    return out
