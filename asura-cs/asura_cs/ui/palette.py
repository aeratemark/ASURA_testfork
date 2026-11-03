"""Object Window: templates to place (Python-defined), the profile's own records, and the models used
by the current map's scenery (read-only, for finding model ids)."""

from __future__ import annotations

from collections import Counter

from PySide6.QtCore import QMimeData, Qt, Signal
from PySide6.QtGui import QDrag
from PySide6.QtWidgets import (QHBoxLayout, QInputDialog, QLineEdit, QMenu, QMessageBox, QPushButton, QTabWidget,
                               QTreeWidget, QTreeWidgetItem, QVBoxLayout, QWidget)

from .. import library
from ..content import commands as C
from ..content.lua_templates import script_template
from ..content.model import AgentDef, Dialogue, DialogueNode, Quest, Script, valid_id
from .viewport2d import MIME_AGENT, MIME_TEMPLATE

ROLE = Qt.UserRole


class _DragTree(QTreeWidget):
    def __init__(self, mime_for):
        super().__init__()
        self.mime_for = mime_for
        self.setDragEnabled(True)
        self.setHeaderHidden(True)
        self.setRootIsDecorated(True)

    def startDrag(self, actions):
        it = self.currentItem()
        if it is None:
            return
        mime = self.mime_for(it)
        if mime is None:
            return
        drag = QDrag(self)
        drag.setMimeData(mime)
        drag.exec(Qt.CopyAction)


def _filter_tree(tree: QTreeWidget, text: str) -> None:
    text = text.lower().strip()

    def visit(item: QTreeWidgetItem) -> bool:
        child_hit = False
        for i in range(item.childCount()):
            child_hit |= visit(item.child(i))
        hit = not text or text in item.text(0).lower() or child_hit
        item.setHidden(not hit)
        if text and child_hit:
            item.setExpanded(True)
        return hit

    for i in range(tree.topLevelItemCount()):
        visit(tree.topLevelItem(i))


class PalettePanel(QWidget):
    openRecord = Signal(object)    # double-clicked record -> editor

    def __init__(self, doc, parent=None):
        super().__init__(parent)
        self.doc = doc
        lay = QVBoxLayout(self)
        lay.setContentsMargins(2, 2, 2, 2)
        self.filter = QLineEdit()
        self.filter.setPlaceholderText("Filter…")
        self.filter.setClearButtonEnabled(True)
        lay.addWidget(self.filter)
        self.tabs = QTabWidget()
        lay.addWidget(self.tabs)

        self.place = _DragTree(self._mime_for_place)
        self.place.itemClicked.connect(self._arm_from_item)
        self.tabs.addTab(self.place, "Place")

        recw = QWidget()
        rl = QVBoxLayout(recw)
        rl.setContentsMargins(0, 0, 0, 0)
        self.records = _DragTree(self._mime_for_record)
        self.records.itemDoubleClicked.connect(self._open_item)
        self.records.itemClicked.connect(self._select_record_item)
        self.records.setContextMenuPolicy(Qt.CustomContextMenu)
        self.records.customContextMenuRequested.connect(self._records_menu)
        rl.addWidget(self.records)
        row = QHBoxLayout()
        for label, kind in (("+Agent", "agent"), ("+Dialogue", "dialogue"), ("+Quest", "quest"), ("+Script", "script")):
            b = QPushButton(label)
            b.clicked.connect(lambda _=False, k=kind: self.new_record(k))
            row.addWidget(b)
        rl.addLayout(row)
        self.tabs.addTab(recw, "Records")

        self.models = QTreeWidget()
        self.models.setHeaderLabels(["Model file id", "Uses"])
        self.models.itemDoubleClicked.connect(self._apply_model_id)
        self.models.setToolTip("Models used by this map's scenery (from Gw.dat / GWMB).\n"
                               "Double-click to assign the id to the selected agent definition or portal.")
        self.tabs.addTab(self.models, "Map models")

        self.filter.textChanged.connect(self._apply_filter)
        doc.profileReset.connect(self.refresh)
        doc.mapChanged.connect(lambda _id: self.refresh_models())
        doc.contentChanged.connect(self._on_content)
        self.refresh()

    # -- building -------------------------------------------------------------------------------
    def refresh(self) -> None:
        dirs = []
        if self.doc.folder:
            dirs.append(self.doc.folder / "library")
        library.reload(dirs)
        for err in library.load_errors():
            self.doc.message.emit("Library error: " + err)
        self.place.clear()
        nodes: dict[str, QTreeWidgetItem] = {}
        for t in library.templates():
            parts = t.path.split("/")
            parent = None
            for depth in range(len(parts) - 1):
                key = "/".join(parts[:depth + 1])
                if key not in nodes:
                    it = QTreeWidgetItem([parts[depth]])
                    it.setFlags(it.flags() & ~Qt.ItemIsDragEnabled)
                    (parent.addChild(it) if parent else self.place.addTopLevelItem(it))
                    it.setExpanded(True)
                    nodes[key] = it
                parent = nodes[key]
            leaf = QTreeWidgetItem([t.name])
            leaf.setData(0, ROLE, ("template", t.path))
            leaf.setToolTip(0, t.description or t.path)
            (parent.addChild(leaf) if parent else self.place.addTopLevelItem(leaf))
        self.refresh_records()
        self.refresh_models()
        self._apply_filter(self.filter.text())

    def refresh_records(self) -> None:
        self.records.clear()
        p = self.doc.profile
        for label, coll, kind in (("Agents", p.agents, "agent"), ("Dialogue", p.dialogues, "dialogue"),
                                  ("Quests", p.quests, "quest"), ("Scripts", p.scripts, "script")):
            top = QTreeWidgetItem([f"{label} ({len(coll)})"])
            self.records.addTopLevelItem(top)
            for r in sorted(coll, key=lambda r: r.id):
                name = getattr(r, "name", "")
                it = QTreeWidgetItem([f"{r.id}" + (f"  —  {name}" if name and name != r.id else "")])
                it.setData(0, ROLE, (kind, r))
                top.addChild(it)
            top.setExpanded(True)
        self._apply_filter(self.filter.text())

    def refresh_models(self) -> None:
        self.models.clear()
        sc = self.doc.scene()
        if sc is None:
            return
        counts: Counter = Counter()
        if sc.gwmb is not None:
            counts.update(p.model_hash for p in sc.gwmb.placements)
        elif sc.ffna is not None:
            counts.update(p.model_file_id for p in sc.ffna.props if p.model_file_id)
        if self.doc.map:
            counts.update(o.model_file_id for o in self.doc.map.objects if getattr(o, "model_file_id", 0))
        for mid, n in counts.most_common():
            it = QTreeWidgetItem([f"0x{mid:X}  ({mid})", str(n)])
            it.setData(0, ROLE, ("model", mid))
            self.models.addTopLevelItem(it)

    def _apply_filter(self, text: str) -> None:
        _filter_tree(self.place, text)
        _filter_tree(self.records, text)
        _filter_tree(self.models, text)

    def _on_content(self, kind, obj) -> None:
        if isinstance(obj, (AgentDef, Dialogue, Quest, Script)):
            self.refresh_records()

    # -- interaction ------------------------------------------------------------------------------
    def _mime_for_place(self, it):
        data = it.data(0, ROLE)
        if not data or data[0] != "template":
            return None
        m = QMimeData()
        m.setData(MIME_TEMPLATE, data[1].encode())
        return m

    def _mime_for_record(self, it):
        data = it.data(0, ROLE)
        if not data or data[0] != "agent":
            return None
        m = QMimeData()
        m.setData(MIME_AGENT, data[1].id.encode())
        return m

    def _arm_from_item(self, it, _col) -> None:
        data = it.data(0, ROLE)
        if data and data[0] == "template":
            t = next((t for t in library.templates() if t.path == data[1]), None)
            self.doc.arm(t)
            if t:
                self.doc.message.emit(f"Click in the Top or 3D view to place '{t.name}'. Right-click/Esc stops.")

    def _select_record_item(self, it, _col) -> None:
        data = it.data(0, ROLE)
        if data and data[0] in ("agent", "dialogue", "quest", "script"):
            self.doc.select([data[1]])

    def _open_item(self, it, _col) -> None:
        data = it.data(0, ROLE)
        if data and data[0] in ("agent", "dialogue", "quest", "script"):
            self.openRecord.emit(data[1])

    def _apply_model_id(self, it, _col) -> None:
        data = it.data(0, ROLE)
        if not data:
            return
        targets = [s for s in self.doc.selection if hasattr(s, "model_file_id")]
        if not targets:
            QMessageBox.information(self, "Model id", "Select an agent definition or a portal first.")
            return
        self.doc.set_fields(targets, {"model_file_id": int(data[1])}, "Assign model")

    def _records_menu(self, pos) -> None:
        it = self.records.itemAt(pos)
        data = it.data(0, ROLE) if it else None
        menu = QMenu(self)
        if data and data[0] in ("agent", "dialogue", "quest", "script"):
            rec = data[1]
            menu.addAction("Open", lambda: self.openRecord.emit(rec))
            menu.addAction("Rename…", lambda: self.rename_record(rec))
            menu.addAction("Delete", lambda: self.delete_record(data[0], rec))
            if data[0] == "agent":
                menu.addAction("Place in map (arm)", lambda: self._arm_agent(rec))
        menu.addSeparator()
        for kind in ("agent", "dialogue", "quest", "script"):
            menu.addAction(f"New {kind}…", lambda k=kind: self.new_record(k))
        menu.exec(self.records.viewport().mapToGlobal(pos))

    def _arm_agent(self, a: AgentDef) -> None:
        tmpl = library.Template(f"Profile/{a.id}", a.name, agent={"id": a.id, "name": a.name}, object_type="placement")
        self.doc.arm(tmpl)

    # -- record management (undoable) --------------------------------------------------------------
    def new_record(self, kind: str):
        p = self.doc.profile
        rid, ok = QInputDialog.getText(self, f"New {kind}", "Editor ID:", text=p.unique_id(f"new_{kind}"))
        if not ok or not rid:
            return None
        if not valid_id(rid) or rid in p.all_ids():
            QMessageBox.warning(self, "Editor ID", f"{rid!r} is not a valid, unused id.")
            return None
        if kind == "agent":
            rec, coll = AgentDef(id=rid, name=rid.replace("_", " ").title()), p.agents
        elif kind == "dialogue":
            rec, coll = Dialogue(id=rid, name=rid, nodes=[DialogueNode("greeting", "Hello, traveller.")]), p.dialogues
        elif kind == "quest":
            rec, coll = Quest(id=rid, name=rid.replace("_", " ").title()), p.quests
        else:
            rec, coll = Script(id=rid, source=script_template(rid)), p.scripts
        self.doc.push(C.AddRecords([(coll, rec)], f"New {kind} {rid}"))
        self.doc.select([rec])
        self.openRecord.emit(rec)
        return rec

    def rename_record(self, rec) -> None:
        new, ok = QInputDialog.getText(self, "Rename", "New editor ID:", text=rec.id)
        if ok and new != rec.id:
            err = self.doc.rename(rec, new)
            if err:
                QMessageBox.warning(self, "Rename", err)
            self.refresh_records()

    def delete_record(self, kind: str, rec) -> None:
        coll = self.doc.profile.collection(kind)
        if QMessageBox.question(self, "Delete", f"Delete {kind} {rec.id}? References to it will be "
                                "reported by Validate.") != QMessageBox.Yes:
            return
        self.doc.push(C.RemoveRecords([(coll, rec)], f"Delete {kind} {rec.id}"))
