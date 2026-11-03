"""Dialogue editor in the TES Construction Set manner: nodes on the left, the selected node's text,
condition and result script on the right, and its choices (the buttons the player sees) in a table.

Every change is one undo step (``Mutate`` snapshots the dialogue). ``Preview`` plays the conversation
without a server: conditions are not evaluated (shown instead), so every branch can be walked.
"""

from __future__ import annotations

from PySide6.QtCore import Qt, QTimer
from PySide6.QtGui import QFont
from PySide6.QtWidgets import (QAbstractItemView, QComboBox, QDialog, QFormLayout, QHBoxLayout, QHeaderView,
                               QInputDialog, QLabel, QLineEdit, QListWidget, QListWidgetItem, QMessageBox,
                               QPlainTextEdit, QPushButton, QSplitter, QTableWidget, QTableWidgetItem, QTextBrowser,
                               QVBoxLayout, QWidget)

from ..content import commands as C
from ..content.model import (CHOICE_KINDS, CHOICE_WIRE_KIND, DIALOG_LINE_UNITS, Dialogue, DialogueChoice,
                             DialogueNode)
from .lua_highlight import LuaHighlighter

COLS = ("Button text", "Kind", "Quest", "Go to node", "Condition (Lua expr)", "Result (Lua)")


def _mono():
    f = QFont("Consolas")
    f.setStyleHint(QFont.Monospace)
    f.setPointSize(9)
    return f


class DialogueEditor(QWidget):
    def __init__(self, doc, parent=None):
        super().__init__(parent)
        self.doc = doc
        self.dialogue: Dialogue | None = None
        self._loading = False

        top = QHBoxLayout()
        self.picker = QComboBox()
        self.picker.setMinimumWidth(220)
        self.picker.activated.connect(lambda _i: self.open(self.picker.currentData()))
        top.addWidget(QLabel("Dialogue:"))
        top.addWidget(self.picker, 1)
        self.start_edit = QLineEdit()
        self.start_edit.setMaximumWidth(140)
        self.start_edit.editingFinished.connect(self._set_start)
        top.addWidget(QLabel("Start node:"))
        top.addWidget(self.start_edit)
        prev = QPushButton("Preview…")
        prev.clicked.connect(self.preview)
        top.addWidget(prev)

        left = QWidget()
        ll = QVBoxLayout(left)
        ll.setContentsMargins(0, 0, 0, 0)
        ll.addWidget(QLabel("Nodes (topics / infos)"))
        self.nodes = QListWidget()
        self.nodes.currentRowChanged.connect(self._show_node)
        ll.addWidget(self.nodes)
        row = QHBoxLayout()
        for text, fn in (("Add", self.add_node), ("Rename", self.rename_node), ("Delete", self.delete_node),
                         ("▲", lambda: self.move_node(-1)), ("▼", lambda: self.move_node(1))):
            b = QPushButton(text)
            b.clicked.connect(fn)
            row.addWidget(b)
        ll.addLayout(row)

        right = QWidget()
        rl = QVBoxLayout(right)
        rl.setContentsMargins(0, 0, 0, 0)
        form = QFormLayout()
        self.text = QPlainTextEdit()
        self.text.setPlaceholderText("What the NPC says. One line per dialog text message "
                                     f"(≤ {DIALOG_LINE_UNITS} characters each).")
        self.text.setFixedHeight(90)
        self.units = QLabel()
        self.text.textChanged.connect(self._count)
        self.condition = QLineEdit()
        self.condition.setFont(_mono())
        self.condition.setPlaceholderText("e.g.  quest.state(character, 'rurik_first_errand') == 'active'")
        self.result = QPlainTextEdit()
        self.result.setFont(_mono())
        self.result.setFixedHeight(60)
        self.result.setPlaceholderText("Lua run when this node is shown")
        LuaHighlighter(self.result.document())
        self.alternatives = QLineEdit()
        self.alternatives.setPlaceholderText("node ids tried in order when the condition fails")
        form.addRow("Text", self.text)
        form.addRow("", self.units)
        form.addRow("Condition", self.condition)
        form.addRow("Result", self.result)
        form.addRow("Fallbacks", self.alternatives)
        rl.addLayout(form)
        for w in (self.condition, self.alternatives):
            w.editingFinished.connect(self._commit_node)
        self._focus_commit(self.text)
        self._focus_commit(self.result)

        rl.addWidget(QLabel("Choices (the buttons)"))
        self.table = QTableWidget(0, len(COLS))
        self.table.setHorizontalHeaderLabels(COLS)
        self.table.horizontalHeader().setSectionResizeMode(QHeaderView.Interactive)
        self.table.horizontalHeader().setStretchLastSection(True)
        self.table.setSelectionBehavior(QAbstractItemView.SelectRows)
        self.table.setColumnWidth(0, 200)
        self.table.itemChanged.connect(self._choice_item_changed)
        rl.addWidget(self.table, 1)
        row = QHBoxLayout()
        for text, fn in (("Add choice", self.add_choice), ("Remove", self.remove_choice),
                         ("▲", lambda: self.move_choice(-1)), ("▼", lambda: self.move_choice(1))):
            b = QPushButton(text)
            b.clicked.connect(fn)
            row.addWidget(b)
        row.addStretch(1)
        rl.addLayout(row)

        split = QSplitter()
        split.addWidget(left)
        split.addWidget(right)
        split.setStretchFactor(1, 3)
        lay = QVBoxLayout(self)
        lay.addLayout(top)
        lay.addWidget(split, 1)

        doc.profileReset.connect(self._refresh_picker)
        doc.contentChanged.connect(self._on_content)
        self._refresh_picker()

    # -- plumbing ----------------------------------------------------------------------------------
    def _focus_commit(self, w):
        from .properties import _FocusOut
        f = _FocusOut(self._commit_node)
        w.installEventFilter(f)
        setattr(self, f"_filter_{id(w)}", f)

    def _refresh_picker(self):
        cur = self.dialogue
        self.picker.clear()
        for d in sorted(self.doc.profile.dialogues, key=lambda d: d.id):
            self.picker.addItem(f"{d.id} — {d.name}", d)
        if cur is not None and cur in self.doc.profile.dialogues:
            self.picker.setCurrentIndex(self.picker.findData(cur))
        else:
            self.open(self.picker.itemData(0) if self.picker.count() else None)

    def _on_content(self, kind, obj):
        if isinstance(obj, Dialogue):
            if kind in ("added", "removed"):
                self._refresh_picker()
            elif obj is self.dialogue and not self._loading:
                QTimer.singleShot(0, lambda: self._reload(keep_row=True))

    def edit(self, fn, text="Edit dialogue"):
        if self.dialogue is not None:
            self._loading = True
            try:
                self.doc.push(C.Mutate(self.dialogue, fn, text))
            finally:
                self._loading = False
            # Deferred: the edit may come from a widget inside the table the reload replaces.
            QTimer.singleShot(0, lambda: self._reload(keep_row=True))

    # -- view -------------------------------------------------------------------------------------
    def open(self, d: Dialogue | None):
        self.dialogue = d
        i = self.picker.findData(d) if d is not None else -1
        if i >= 0:
            self.picker.setCurrentIndex(i)
        self._reload()

    def _reload(self, keep_row=False):
        row = self.nodes.currentRow() if keep_row else 0
        self._loading = True
        self.nodes.clear()
        d = self.dialogue
        self.setEnabled(True)
        if d is not None:
            self.start_edit.setText(d.start)
            for n in d.nodes:
                it = QListWidgetItem(("★ " if n.id == d.start else "") + n.id)
                it.setToolTip(n.text[:200])
                self.nodes.addItem(it)
        self._loading = False
        if d is not None and d.nodes:
            self.nodes.setCurrentRow(min(max(row, 0), len(d.nodes) - 1))
        else:
            self._show_node(-1)

    def node(self) -> DialogueNode | None:
        r = self.nodes.currentRow()
        if self.dialogue is None or not 0 <= r < len(self.dialogue.nodes):
            return None
        return self.dialogue.nodes[r]

    def _show_node(self, row):
        self._loading = True
        n = self.node()
        for w in (self.text, self.result):
            w.setPlainText(getattr(n, "text" if w is self.text else "result", "") if n else "")
        self.condition.setText(n.condition if n else "")
        self.alternatives.setText(", ".join(n.alternatives) if n else "")
        self.table.setRowCount(0)
        if n:
            ids = [x.id for x in self.dialogue.nodes]
            quests = [q.id for q in self.doc.profile.quests]
            for c in n.choices:
                r = self.table.rowCount()
                self.table.insertRow(r)
                self.table.setItem(r, 0, QTableWidgetItem(c.text))
                kind = QComboBox()
                kind.addItems(CHOICE_KINDS)
                kind.setCurrentText(c.kind)
                kind.setToolTip("Decides the icon the client draws (quest kinds: Rurik measurements).")
                kind.activated.connect(lambda _i, r=r, w=kind: self._set_choice(r, "kind", w.currentText()))
                self.table.setCellWidget(r, 1, kind)
                q = QComboBox()
                q.addItems([""] + quests)
                q.setCurrentText(c.quest)
                q.setEnabled(c.kind in CHOICE_WIRE_KIND)
                q.activated.connect(lambda _i, r=r, w=q: self._set_choice(r, "quest", w.currentText()))
                self.table.setCellWidget(r, 2, q)
                go = QComboBox()
                go.addItems(["end"] + ids)
                go.setCurrentText(c.goto or "end")
                go.activated.connect(lambda _i, r=r, w=go: self._set_choice(r, "goto", w.currentText()))
                self.table.setCellWidget(r, 3, go)
                cond = QTableWidgetItem(c.condition)
                cond.setFont(_mono())
                self.table.setItem(r, 4, cond)
                res = QTableWidgetItem(c.result)
                res.setFont(_mono())
                self.table.setItem(r, 5, res)
        self._count()
        self._loading = False

    def _count(self):
        lines = self.text.toPlainText().splitlines() or [""]
        longest = max(len(x.encode("utf-16-le")) // 2 for x in lines)
        over = longest > DIALOG_LINE_UNITS
        self.units.setText(f"{len(lines)} line(s), longest {longest}/{DIALOG_LINE_UNITS}"
                           + ("  — too long for one dialog line" if over else ""))
        self.units.setStyleSheet("color: #a00000;" if over else "color: #404040;")

    # -- node edits ---------------------------------------------------------------------------------
    def _commit_node(self):
        n = self.node()
        if n is None or self._loading:
            return
        text, cond, res = self.text.toPlainText(), self.condition.text(), self.result.toPlainText()
        alts = [a.strip() for a in self.alternatives.text().split(",") if a.strip()]
        if (text, cond, res, alts) == (n.text, n.condition, n.result, n.alternatives):
            return
        idx = self.nodes.currentRow()

        def fn(d):
            nd = d.nodes[idx]
            nd.text, nd.condition, nd.result, nd.alternatives = text, cond, res, alts
        self.edit(fn, f"Edit node {n.id}")

    def _set_start(self):
        if self.dialogue is None or self._loading:
            return
        v = self.start_edit.text().strip()
        if v and v != self.dialogue.start:
            self.edit(lambda d: setattr(d, "start", v), "Set start node")

    def add_node(self):
        if self.dialogue is None:
            return
        nid, ok = QInputDialog.getText(self, "New node", "Node id:", text=f"node{len(self.dialogue.nodes) + 1}")
        if not ok or not nid.strip():
            return
        nid = nid.strip()
        if any(n.id == nid for n in self.dialogue.nodes):
            QMessageBox.warning(self, "Node", f"A node {nid!r} already exists.")
            return
        self.edit(lambda d: d.nodes.append(DialogueNode(nid, "")), f"Add node {nid}")
        last = len(self.dialogue.nodes) - 1
        QTimer.singleShot(0, lambda: self.nodes.setCurrentRow(last))

    def rename_node(self):
        n = self.node()
        if n is None:
            return
        new, ok = QInputDialog.getText(self, "Rename node", "Node id:", text=n.id)
        if not ok or not new.strip() or new == n.id:
            return
        old, new = n.id, new.strip()

        def fn(d):
            for x in d.nodes:
                if x.id == old:
                    x.id = new
                x.alternatives = [new if a == old else a for a in x.alternatives]
                for c in x.choices:
                    if c.goto == old:
                        c.goto = new
            if d.start == old:
                d.start = new
        self.edit(fn, f"Rename node {old}")

    def delete_node(self):
        idx = self.nodes.currentRow()
        if self.node() is None:
            return
        self.edit(lambda d: d.nodes.pop(idx), "Delete node")

    def move_node(self, delta):
        i = self.nodes.currentRow()
        j = i + delta
        if self.dialogue is None or not (0 <= i < len(self.dialogue.nodes) and 0 <= j < len(self.dialogue.nodes)):
            return

        def fn(d):
            d.nodes[i], d.nodes[j] = d.nodes[j], d.nodes[i]
        self.edit(fn, "Reorder nodes")
        QTimer.singleShot(0, lambda: self.nodes.setCurrentRow(j))

    # -- choice edits -------------------------------------------------------------------------------
    def _set_choice(self, row, field, value):
        idx = self.nodes.currentRow()
        if field == "goto" and value == "end":
            value = "end"

        def fn(d):
            setattr(d.nodes[idx].choices[row], field, value)
        self.edit(fn, f"Set choice {field}")

    def _choice_item_changed(self, item):
        if self._loading:
            return
        field = {0: "text", 4: "condition", 5: "result"}.get(item.column())
        if field:
            self._set_choice(item.row(), field, item.text())

    def add_choice(self):
        idx = self.nodes.currentRow()
        if self.node() is None:
            return
        self.edit(lambda d: d.nodes[idx].choices.append(DialogueChoice("Goodbye.", "end")), "Add choice")

    def remove_choice(self):
        idx, r = self.nodes.currentRow(), self.table.currentRow()
        n = self.node()
        if n is None or not 0 <= r < len(n.choices):
            return
        self.edit(lambda d: d.nodes[idx].choices.pop(r), "Remove choice")

    def move_choice(self, delta):
        idx, r = self.nodes.currentRow(), self.table.currentRow()
        n = self.node()
        if n is None or not (0 <= r < len(n.choices) and 0 <= r + delta < len(n.choices)):
            return

        def fn(d):
            ch = d.nodes[idx].choices
            ch[r], ch[r + delta] = ch[r + delta], ch[r]
        self.edit(fn, "Reorder choices")
        QTimer.singleShot(0, lambda: self.table.setCurrentCell(r + delta, 0))

    # -- preview -------------------------------------------------------------------------------------
    def preview(self):
        if self.dialogue is None or not self.dialogue.nodes:
            return
        DialoguePreview(self.dialogue, self).exec()


class DialoguePreview(QDialog):
    """Walk the conversation like the in-game dialog window: text, then one button per choice."""

    def __init__(self, d: Dialogue, parent=None):
        super().__init__(parent)
        self.d = d
        self.setWindowTitle(f"Preview — {d.name}")
        self.resize(460, 360)
        lay = QVBoxLayout(self)
        self.view = QTextBrowser()
        self.view.setStyleSheet("background: #1c1a16; color: #e8dcc0; font-size: 10pt;")
        lay.addWidget(self.view, 1)
        self.buttons = QVBoxLayout()
        lay.addLayout(self.buttons)
        self.show_node(d.start)

    def show_node(self, nid):
        node = next((n for n in self.d.nodes if n.id == nid), None)
        while self.buttons.count():
            w = self.buttons.takeAt(0).widget()
            if w is not None:
                w.deleteLater()
        if node is None:
            self.accept()
            return
        html = "<br>".join(line for line in node.text.splitlines()) or "<i>(no text)</i>"
        if node.condition:
            html += f"<p style='color:#8a8'><small>shown if: {node.condition}</small></p>"
        if node.result:
            html += f"<p style='color:#88a'><small>runs: {node.result}</small></p>"
        self.view.setHtml(html)
        icons = {"quest_available": "! ", "accept": "✔ ", "decline": "⊘ ", "advance": "• ",
                 "in_progress": "? ", "turn_in": "◆ "}
        for c in node.choices or [DialogueChoice("(close)", "end")]:
            b = QPushButton(icons.get(c.kind, "") + c.text)
            if c.condition:
                b.setToolTip(f"if {c.condition}")
            b.clicked.connect(lambda _=False, g=c.goto: self.show_node(g) if g and g != "end" else self.accept())
            self.buttons.addWidget(b)


__all__ = ["DialogueEditor", "Qt"]
