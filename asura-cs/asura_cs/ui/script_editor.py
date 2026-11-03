"""Lua script editor (TES CS "Script Edit" window): pick a script, edit, check, save to the profile.

Text changes are buffered; ``Apply`` (Ctrl+S inside the editor) or leaving the editor commits them as
one undo step, so the undo history is not one entry per keystroke.
"""

from __future__ import annotations

from PySide6.QtCore import Qt
from PySide6.QtGui import QFont, QKeySequence, QShortcut, QTextCursor
from PySide6.QtWidgets import (QComboBox, QHBoxLayout, QInputDialog, QLabel, QListWidget, QMessageBox,
                               QPlainTextEdit, QPushButton, QSplitter, QVBoxLayout, QWidget)

from ..content import commands as C
from ..content.lua_templates import API, EVENTS, script_template
from ..content.model import Script, valid_id
from ..content.validate import find_luac, lua_syntax_error
from .lua_highlight import LuaHighlighter


class ScriptEditor(QWidget):
    def __init__(self, doc, parent=None):
        super().__init__(parent)
        self.doc = doc
        self.script: Script | None = None
        self._loading = False

        top = QHBoxLayout()
        self.picker = QComboBox()
        self.picker.setMinimumWidth(220)
        self.picker.activated.connect(lambda _i: self.open(self.picker.currentData()))
        top.addWidget(QLabel("Script:"))
        top.addWidget(self.picker, 1)
        for text, fn in (("New…", self.new_script), ("Apply", self.apply), ("Revert", self.revert),
                         ("Check syntax", self.check)):
            b = QPushButton(text)
            b.clicked.connect(fn)
            top.addWidget(b)

        self.edit = QPlainTextEdit()
        f = QFont("Consolas")
        f.setStyleHint(QFont.Monospace)
        f.setPointSize(10)
        self.edit.setFont(f)
        self.edit.setLineWrapMode(QPlainTextEdit.NoWrap)
        self.edit.setTabStopDistance(self.edit.fontMetrics().horizontalAdvance(" ") * 4)
        LuaHighlighter(self.edit.document())
        self.edit.textChanged.connect(self._changed)
        sc = QShortcut(QKeySequence(QKeySequence.Save), self.edit)
        sc.activated.connect(self.apply)
        sc.setContext(Qt.WidgetShortcut)  # Ctrl+S in the editor applies; elsewhere it saves the profile

        self.help = QListWidget()
        self.help.setToolTip("Double-click to insert. Event and API names are a proposal until ASURA "
                             "fixes its Lua API (docs/30).")
        for kind, events in EVENTS.items():
            for name, args, doc_ in events:
                self.help.addItem(f"function M.{name}({args})  -- {kind}: {doc_}")
        for root, names in API.items():
            for n in names:
                self.help.addItem(f"{root}.{n}")
        self.help.itemDoubleClicked.connect(self._insert)

        split = QSplitter()
        split.addWidget(self.edit)
        split.addWidget(self.help)
        split.setStretchFactor(0, 4)
        self.status = QLabel()
        lay = QVBoxLayout(self)
        lay.addLayout(top)
        lay.addWidget(split, 1)
        lay.addWidget(self.status)

        doc.profileReset.connect(self._refresh_picker)
        doc.contentChanged.connect(self._on_content)
        self._refresh_picker()

    def _refresh_picker(self):
        cur = self.script
        self.picker.clear()
        for s in sorted(self.doc.profile.scripts, key=lambda s: s.id):
            self.picker.addItem(s.id, s)
        if cur is not None and cur in self.doc.profile.scripts:
            self.picker.setCurrentIndex(self.picker.findData(cur))
        else:
            self.open(self.picker.itemData(0) if self.picker.count() else None, ask=False)

    def _on_content(self, kind, obj):
        if isinstance(obj, Script):
            if kind in ("added", "removed"):
                self._refresh_picker()
            elif obj is self.script and not self._loading and not self.pending():
                self._load_text()

    def pending(self) -> bool:
        return self.script is not None and self.edit.toPlainText() != self.script.source

    def open(self, s: Script | None, ask: bool = True):
        if ask and self.pending():
            r = QMessageBox.question(self, "Script", f"Apply changes to {self.script.id}?",
                                     QMessageBox.Yes | QMessageBox.No | QMessageBox.Cancel)
            if r == QMessageBox.Cancel:
                self.picker.setCurrentIndex(self.picker.findData(self.script))
                return
            if r == QMessageBox.Yes:
                self.apply()
        self.script = s
        if s is not None:
            self.picker.setCurrentIndex(self.picker.findData(s))
        self._load_text()

    def _load_text(self):
        self._loading = True
        self.edit.setPlainText(self.script.source if self.script else "")
        self.edit.setEnabled(self.script is not None)
        self._loading = False
        self._changed()

    def _changed(self):
        if self.script is None:
            self.status.setText("No script. Use New… to create one.")
            return
        self.status.setText(("● modified (Apply or Ctrl+S)" if self.pending() else "saved in profile")
                            + ("" if find_luac() else "   |   install luac for syntax checks"))

    def apply(self):
        if not self.pending():
            return
        text = self.edit.toPlainText()
        self._loading = True
        try:
            self.doc.push(C.Mutate(self.script, lambda s: setattr(s, "source", text), f"Edit script {self.script.id}"))
        finally:
            self._loading = False
        self._changed()

    def revert(self):
        self._load_text()

    def check(self):
        if not find_luac():
            QMessageBox.information(self, "Check syntax", "No luac found on PATH (install Lua 5.4).")
            return
        err = lua_syntax_error(self.edit.toPlainText())
        if err:
            self.status.setText(f"✗ {err}")
            import re
            m = re.match(r"line (\d+):", err)
            if m:
                block = self.edit.document().findBlockByLineNumber(int(m.group(1)) - 1)
                cur = self.edit.textCursor()
                cur.setPosition(block.position())
                cur.movePosition(QTextCursor.EndOfBlock, QTextCursor.KeepAnchor)
                self.edit.setTextCursor(cur)
        else:
            self.status.setText("✓ syntax OK")

    def new_script(self):
        p = self.doc.profile
        sid, ok = QInputDialog.getText(self, "New script", "Script id:", text=p.unique_id("new_script"))
        if not ok:
            return
        if not valid_id(sid) or sid in p.all_ids():
            QMessageBox.warning(self, "Script", f"{sid!r} is not a valid, unused id.")
            return
        kind, ok = QInputDialog.getItem(self, "New script", "Template:", list(EVENTS), 0, False)
        if not ok:
            return
        s = Script(id=sid, source=script_template(sid, kind))
        self.doc.push(C.AddRecords([(p.scripts, s)], f"New script {sid}"))
        self.open(s, ask=False)

    def _insert(self, item):
        text = item.text().split("  --")[0]
        if text.startswith("function "):
            text += "\nend\n"
        self.edit.insertPlainText(text)
        self.edit.setFocus()
