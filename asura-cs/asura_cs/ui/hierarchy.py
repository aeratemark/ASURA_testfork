"""World hierarchy: every object of the current map, grouped and filterable, synced with selection."""

from __future__ import annotations

from collections import defaultdict

from PySide6.QtCore import Qt, Signal
from PySide6.QtWidgets import (QAbstractItemView, QComboBox, QHBoxLayout, QLabel, QLineEdit, QTreeWidget,
                               QTreeWidgetItem, QVBoxLayout, QWidget)

from ..content.model import AgentPlacement, MapObject, Route, Group, Waypoint

ROLE = Qt.UserRole
GROUPINGS = ("Type", "Layer", "Group", "Route", "Agent definition")


class HierarchyPanel(QWidget):
    focusRequested = Signal(object)

    def __init__(self, doc, parent=None):
        super().__init__(parent)
        self.doc = doc
        self._syncing = False
        lay = QVBoxLayout(self)
        lay.setContentsMargins(2, 2, 2, 2)
        row = QHBoxLayout()
        self.filter = QLineEdit()
        self.filter.setPlaceholderText("Filter by name, id, type…")
        self.filter.setClearButtonEnabled(True)
        self.group = QComboBox()
        self.group.addItems(GROUPINGS)
        row.addWidget(self.filter, 1)
        row.addWidget(QLabel("by"))
        row.addWidget(self.group)
        lay.addLayout(row)
        self.tree = QTreeWidget()
        self.tree.setHeaderLabels(["Name", "Id", "Type"])
        self.tree.setSelectionMode(QAbstractItemView.ExtendedSelection)
        self.tree.setColumnWidth(0, 170)
        self.tree.setColumnWidth(1, 140)
        self.tree.setSortingEnabled(True)
        self.tree.sortByColumn(0, Qt.AscendingOrder)
        lay.addWidget(self.tree)
        self.info = QLabel()
        lay.addWidget(self.info)

        self.filter.textChanged.connect(self.rebuild)
        self.group.currentIndexChanged.connect(self.rebuild)
        self.tree.itemSelectionChanged.connect(self._tree_selection)
        self.tree.itemDoubleClicked.connect(self._double)
        doc.mapChanged.connect(lambda _id: self.rebuild())
        doc.profileReset.connect(self.rebuild)
        doc.contentChanged.connect(self._on_content)
        doc.selectionChanged.connect(self._doc_selection)

    def _key(self, o: MapObject) -> str:
        g = self.group.currentText()
        if g == "Type":
            return o.TYPE_LABEL + "s"
        if g == "Layer":
            return o.layer or "(no layer)"
        if g == "Group":
            return getattr(o, "group_id", "") or "(no group)"
        if g == "Route":
            return getattr(o, "route", "") or "(no route)"
        return getattr(o, "agent", "") or "(not an agent)"

    def rebuild(self) -> None:
        self._syncing = True
        self.tree.setSortingEnabled(False)
        self.tree.clear()
        m = self.doc.map
        if m is None:
            self._syncing = False
            return
        text = self.filter.text().lower().strip()
        buckets: dict[str, list] = defaultdict(list)
        for o in m.objects:
            hay = f"{o.display_name()} {o.id} {o.TYPE_LABEL} {getattr(o, 'agent', '')} {o.layer}".lower()
            if text and text not in hay:
                continue
            buckets[self._key(o)].append(o)
        root = QTreeWidgetItem([m.label(), str(m.map_id), "instance"])
        self.tree.addTopLevelItem(root)
        for key in sorted(buckets):
            node = QTreeWidgetItem([f"{key} ({len(buckets[key])})", "", ""])
            root.addChild(node)
            for o in buckets[key]:
                it = QTreeWidgetItem([o.display_name(), o.id, o.TYPE_LABEL])
                it.setData(0, ROLE, o)
                node.addChild(it)
            node.setExpanded(True)
        if m.routes or m.groups:
            defs = QTreeWidgetItem(["Routes & groups", "", ""])
            root.addChild(defs)
            for r in m.routes:
                it = QTreeWidgetItem([r.name or r.id, r.id, f"route ({len(m.waypoints(r.id))} wp)"])
                it.setData(0, ROLE, r)
                defs.addChild(it)
            for g in m.groups:
                n = sum(1 for o in m.objects if isinstance(o, AgentPlacement) and o.group_id == g.id)
                it = QTreeWidgetItem([g.name or g.id, g.id, f"group ({n})"])
                it.setData(0, ROLE, g)
                defs.addChild(it)
        sc = self.doc.scene()
        if sc is not None:
            n_props = len(sc.gwmb.placements) if sc.gwmb else (len(sc.ffna.props) if sc.ffna else 0)
            ro = QTreeWidgetItem([f"Scenery (read-only): {n_props} props", "", "game data"])
            ro.setDisabled(True)
            root.addChild(ro)
            if sc.path_graph is not None:
                traps = sum(len(p.trapezoids) for p in sc.path_graph.planes)
                pg = QTreeWidgetItem([f"Path graph (read-only): {len(sc.path_graph.planes)} planes, {traps} trapezoids",
                                      "", "game data"])
                pg.setDisabled(True)
                root.addChild(pg)
        root.setExpanded(True)
        self.tree.setSortingEnabled(True)
        self.info.setText(f"{len(m.objects)} objects")
        self._syncing = False
        self._doc_selection(self.doc.selection)

    def _on_content(self, kind, obj) -> None:
        if isinstance(obj, (MapObject, Route, Group)) or hasattr(obj, "map_id"):
            if kind == "changed" and isinstance(obj, MapObject):
                # cheap path: update the row text
                for it in self._items():
                    if it.data(0, ROLE) is obj:
                        it.setText(0, obj.display_name())
                        it.setText(1, obj.id)
                        return
            self.rebuild()

    def _items(self):
        stack = [self.tree.topLevelItem(i) for i in range(self.tree.topLevelItemCount())]
        while stack:
            it = stack.pop()
            yield it
            stack.extend(it.child(i) for i in range(it.childCount()))

    def _tree_selection(self) -> None:
        if self._syncing:
            return
        objs = [it.data(0, ROLE) for it in self.tree.selectedItems() if it.data(0, ROLE) is not None]
        self._syncing = True
        self.doc.select(objs)
        self._syncing = False

    def _doc_selection(self, sel) -> None:
        if self._syncing:
            return
        self._syncing = True
        ids = {id(s) for s in sel}
        first = None
        for it in self._items():
            on = id(it.data(0, ROLE)) in ids if it.data(0, ROLE) is not None else False
            it.setSelected(on)
            if on and first is None:
                first = it
        if first is not None:
            self.tree.scrollToItem(first)
        self._syncing = False

    def _double(self, it, _col) -> None:
        o = it.data(0, ROLE)
        if isinstance(o, MapObject):
            self.focusRequested.emit(o)
        elif isinstance(o, Route) and self.doc.map:
            self.doc.select(self.doc.map.waypoints(o.id))


__all__ = ["HierarchyPanel", "Waypoint"]
