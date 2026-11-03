"""Quest editor: header fields live in the property panel (the quest is selected when opened);
this tab edits the objective list, which has no flat-field form."""

from __future__ import annotations

from PySide6.QtCore import QTimer
from PySide6.QtWidgets import (QAbstractItemView, QCheckBox, QComboBox, QHBoxLayout, QLabel, QPushButton,
                               QSpinBox, QTableWidget, QTableWidgetItem, QVBoxLayout, QWidget)

from ..content import commands as C
from ..content.model import OBJECTIVE_KINDS, Objective, Quest

COLS = ("Kind", "Target id", "Count", "Description", "Optional")


class QuestEditor(QWidget):
    def __init__(self, doc, parent=None):
        super().__init__(parent)
        self.doc = doc
        self.quest: Quest | None = None
        self._loading = False
        top = QHBoxLayout()
        self.picker = QComboBox()
        self.picker.setMinimumWidth(220)
        self.picker.activated.connect(lambda _i: self.open(self.picker.currentData()))
        top.addWidget(QLabel("Quest:"))
        top.addWidget(self.picker, 1)
        sel = QPushButton("Edit header in Properties")
        sel.clicked.connect(lambda: self.quest and self.doc.select([self.quest]))
        top.addWidget(sel)
        self.table = QTableWidget(0, len(COLS))
        self.table.setHorizontalHeaderLabels(COLS)
        self.table.horizontalHeader().setStretchLastSection(False)
        self.table.setColumnWidth(1, 180)
        self.table.setColumnWidth(3, 320)
        self.table.setSelectionBehavior(QAbstractItemView.SelectRows)
        self.table.itemChanged.connect(self._item_changed)
        row = QHBoxLayout()
        for text, fn in (("Add objective", self.add), ("Remove", self.remove),
                         ("▲", lambda: self.move(-1)), ("▼", lambda: self.move(1))):
            b = QPushButton(text)
            b.clicked.connect(fn)
            row.addWidget(b)
        row.addStretch(1)
        hint = QLabel("Targets: talk/escort/accompany → agent or placement id · kill → agent/group id · "
                      "reach → area id · fetch → item name. Objectives run in order; Lua handles the rest.")
        hint.setWordWrap(True)
        lay = QVBoxLayout(self)
        lay.addLayout(top)
        lay.addWidget(self.table, 1)
        lay.addLayout(row)
        lay.addWidget(hint)
        doc.profileReset.connect(self._refresh_picker)
        doc.contentChanged.connect(self._on_content)
        self._refresh_picker()

    def _refresh_picker(self):
        cur = self.quest
        self.picker.clear()
        for q in sorted(self.doc.profile.quests, key=lambda q: q.id):
            self.picker.addItem(f"{q.id} — {q.name}", q)
        if cur is not None and cur in self.doc.profile.quests:
            self.picker.setCurrentIndex(self.picker.findData(cur))
        else:
            self.open(self.picker.itemData(0) if self.picker.count() else None)

    def _on_content(self, kind, obj):
        if isinstance(obj, Quest):
            if kind in ("added", "removed"):
                self._refresh_picker()
            elif obj is self.quest and not self._loading:
                QTimer.singleShot(0, self._reload)

    def open(self, q: Quest | None):
        self.quest = q
        if q is not None:
            self.picker.setCurrentIndex(self.picker.findData(q))
        self._reload()

    def _reload(self):
        self._loading = True
        self.table.setRowCount(0)
        for r, o in enumerate(self.quest.objectives if self.quest else []):
            self.table.insertRow(r)
            k = QComboBox()
            k.addItems(OBJECTIVE_KINDS)
            k.setCurrentText(o.kind)
            k.activated.connect(lambda _i, r=r, w=k: self._set(r, "kind", w.currentText()))
            self.table.setCellWidget(r, 0, k)
            self.table.setItem(r, 1, QTableWidgetItem(o.target))
            c = QSpinBox()
            c.setRange(1, 9999)
            c.setValue(o.count)
            c.editingFinished.connect(lambda r=r, w=c: self._set(r, "count", w.value()))
            self.table.setCellWidget(r, 2, c)
            self.table.setItem(r, 3, QTableWidgetItem(o.description))
            opt = QCheckBox()
            opt.setChecked(o.optional)
            opt.clicked.connect(lambda on, r=r: self._set(r, "optional", bool(on)))
            self.table.setCellWidget(r, 4, opt)
        self._loading = False

    def _edit(self, fn, text):
        if self.quest is None:
            return
        self._loading = True
        try:
            self.doc.push(C.Mutate(self.quest, fn, text))
        finally:
            self._loading = False
        QTimer.singleShot(0, self._reload)

    def _set(self, row, field, value):
        if self.quest is None or getattr(self.quest.objectives[row], field) == value:
            return
        self._edit(lambda q: setattr(q.objectives[row], field, value), f"Set objective {field}")

    def _item_changed(self, item):
        if self._loading:
            return
        field = {1: "target", 3: "description"}.get(item.column())
        if field:
            self._set(item.row(), field, item.text())

    def add(self):
        self._edit(lambda q: q.objectives.append(Objective()), "Add objective")

    def remove(self):
        r = self.table.currentRow()
        if self.quest and 0 <= r < len(self.quest.objectives):
            self._edit(lambda q: q.objectives.pop(r), "Remove objective")

    def move(self, d):
        r = self.table.currentRow()
        if self.quest and 0 <= r < len(self.quest.objectives) and 0 <= r + d < len(self.quest.objectives):
            def fn(q):
                q.objectives[r], q.objectives[r + d] = q.objectives[r + d], q.objectives[r]
            self._edit(fn, "Reorder objectives")
