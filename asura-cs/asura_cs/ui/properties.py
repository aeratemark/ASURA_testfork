"""Property panel built from field metadata (``content.model.meta``). Every edit is an undo step."""

from __future__ import annotations

from dataclasses import fields as dc_fields

from PySide6.QtCore import QEvent, QObject, Qt, QTimer, Signal
from PySide6.QtGui import QFont
from PySide6.QtWidgets import (QCheckBox, QComboBox, QDoubleSpinBox, QFormLayout, QGroupBox, QHBoxLayout, QLabel,
                               QLineEdit, QMenu, QPlainTextEdit, QPushButton, QScrollArea, QSpinBox, QToolButton,
                               QVBoxLayout, QWidget)

from ..content.model import MapObject, editable_fields

MIXED = "(mixed)"


class _FocusOut(QObject):
    def __init__(self, cb):
        super().__init__()
        self.cb = cb

    def eventFilter(self, obj, ev):
        if ev.type() == QEvent.FocusOut:
            self.cb()
        return False


def _mono() -> QFont:
    f = QFont("Consolas")
    f.setStyleHint(QFont.Monospace)
    f.setPointSize(9)
    return f


class PropertyPanel(QWidget):
    openRecord = Signal(object)

    def __init__(self, doc, parent=None):
        super().__init__(parent)
        self.doc = doc
        self.objs: list = []
        self._filters = []
        outer = QVBoxLayout(self)
        outer.setContentsMargins(2, 2, 2, 2)
        self.title = QLabel("Nothing selected")
        self.title.setStyleSheet("font-weight: bold; padding: 2px;")
        outer.addWidget(self.title)
        self.scroll = QScrollArea()
        self.scroll.setWidgetResizable(True)
        outer.addWidget(self.scroll)
        self.body = QWidget()
        self.scroll.setWidget(self.body)
        doc.selectionChanged.connect(self.show_objects)
        doc.contentChanged.connect(self._on_content)
        self._building = False
        self._built_for: list = []
        # Rebuilds are deferred: an edit often originates from a widget this panel is about to replace,
        # and deleting a widget inside its own signal is unsafe.
        self._pending: tuple | None = None
        self._timer = QTimer(self)
        self._timer.setSingleShot(True)
        self._timer.setInterval(0)
        self._timer.timeout.connect(self._flush)

    # ---------------------------------------------------------------------------------------------
    def _on_content(self, kind, obj) -> None:
        if kind == "changed" and any(obj is o for o in self.objs) and not self._building:
            self.show_objects(self.objs, keep_scroll=True)
        elif kind == "removed" and any(obj is o for o in self.objs):
            self.show_objects([o for o in self.objs if o is not obj])

    def show_objects(self, objs: list, keep_scroll: bool = False) -> None:
        self.objs = list(objs)
        self._pending = (list(objs), keep_scroll)
        self._timer.start()

    def _flush(self) -> None:
        if self._pending is None:
            return
        objs, keep_scroll = self._pending
        self._pending = None
        self._rebuild(objs, keep_scroll)

    def _rebuild(self, objs: list, keep_scroll: bool = False) -> None:
        focus = self.focusWidget()
        focus_name = focus.objectName() if focus is not None and self.isAncestorOf(focus) else ""
        scroll = self.scroll.verticalScrollBar().value()
        self._building = True
        self.objs = list(objs)
        self._built_for = list(objs)
        old_filters = list(self._filters)
        body = QWidget()
        lay = QVBoxLayout(body)
        lay.setContentsMargins(2, 2, 2, 2)
        if not objs:
            self.title.setText("Nothing selected")
        elif len({type(o) for o in objs}) > 1:
            self.title.setText(f"{len(objs)} objects of different types")
            if all(isinstance(o, MapObject) for o in objs):
                self._build_fields(lay, [f for f in dc_fields(MapObject) if f.metadata and f.name in
                                         ("layer", "z", "yaw", "plane")])
        else:
            o = objs[0]
            n = f" × {len(objs)}" if len(objs) > 1 else ""
            self.title.setText(f"{o.TYPE_LABEL}{n}: {o.display_name() if len(objs) == 1 else ''}")
            flds = editable_fields(o)
            if len(objs) > 1:
                flds = [f for f in flds if f.name != "id"]
            self._build_fields(lay, flds)
            if len(objs) == 1 and o.TYPE in ("agent", "dialogue", "quest", "script"):
                b = QPushButton(f"Open {o.TYPE_LABEL.lower()} editor…")
                b.clicked.connect(lambda: self.openRecord.emit(o))
                lay.addWidget(b)
            if len(objs) == 1 and getattr(o, "agent", ""):
                adef = self.doc.profile.find("agent", o.agent)
                if adef is not None:
                    b = QPushButton(f"Edit agent definition '{adef.id}'…")
                    b.clicked.connect(lambda: self.doc.select([adef]))
                    lay.addWidget(b)
        lay.addStretch(1)
        self._building = False
        old = self.scroll.takeWidget()
        self.scroll.setWidget(body)
        if old is not None:
            old.hide()          # a focused text box commits on focus-out here, to its own objects
            old.deleteLater()
        self._filters = [f for f in self._filters if f not in old_filters]
        self.body = body
        if keep_scroll:
            self.scroll.verticalScrollBar().setValue(scroll)
        if focus_name:
            w = body.findChild(QWidget, focus_name)
            if w is not None:
                w.setFocus()
        self._building = False

    # ---------------------------------------------------------------------------------------------
    def _value(self, name):
        vals = [getattr(o, name) for o in self.objs]
        first = vals[0]
        return first if all(v == first for v in vals) else MIXED

    def _committer(self):
        built = list(self._built_for)
        return lambda name, value: self._commit(name, value, built)

    def _commit(self, name, value, objs) -> None:
        if self._building:
            return
        targets = [o for o in objs if getattr(o, name) != value]
        if not targets:
            return
        if name == "id":
            err = self.doc.rename(targets[0], value)
            if err:
                self.doc.message.emit(err)
                self.show_objects(self.objs)
            return
        self.doc.set_fields(targets, {name: value}, f"Set {name}")

    def _build_fields(self, lay, flds) -> None:
        groups: dict[str, QFormLayout] = {}
        for f in flds:
            md = f.metadata
            g = md.get("group", "General")
            if g not in groups:
                box = QGroupBox(g)
                form = QFormLayout(box)
                form.setLabelAlignment(Qt.AlignRight)
                form.setFieldGrowthPolicy(QFormLayout.AllNonFixedFieldsGrow)
                lay.addWidget(box)
                groups[g] = form
            w = self._editor(f.name, md)
            w.setObjectName(f"prop_{f.name}")
            if md.get("tip"):
                w.setToolTip(md["tip"])
            if md.get("readonly"):
                w.setEnabled(False)
            groups[g].addRow(md.get("label") or f.name, w)

    def _editor(self, name: str, md: dict) -> QWidget:
        commit = self._committer()
        kind = md.get("kind", "str")
        v = self._value(name)
        mixed = v == MIXED
        if kind in ("int", "model"):
            if kind == "model":
                e = QLineEdit("" if mixed else (f"0x{v:X}" if v else "0"))
                e.setToolTip(md.get("tip", "") + "\nHex (0x...) or decimal.")

                def done(e=e):
                    try:
                        commit(name, int(e.text().strip() or "0", 0))
                    except ValueError:
                        self.doc.message.emit(f"{name}: not a number")
                e.editingFinished.connect(done)
                return e
            e = QSpinBox()
            e.setRange(-2_000_000_000, 2_000_000_000)
            if mixed:
                e.setSpecialValueText(MIXED)
            else:
                e.setValue(int(v))
            e.editingFinished.connect(lambda e=e: commit(name, e.value()))
            return e
        if kind in ("float", "angle"):
            e = QDoubleSpinBox()
            if kind == "angle":
                e.setRange(-360.0, 720.0)
                e.setWrapping(True)
                e.setSingleStep(15.0)
                e.setSuffix("°")
            else:
                e.setRange(-1e7, 1e7)
                e.setSingleStep(10.0)
            e.setDecimals(2)
            e.setKeyboardTracking(False)
            if not mixed:
                e.setValue(float(v))
            e.editingFinished.connect(lambda e=e: commit(name, float(e.value()) % 360.0
                                                               if kind == "angle" else float(e.value())))
            return e
        if kind == "bool":
            e = QCheckBox()
            if mixed:
                e.setTristate(True)
                e.setCheckState(Qt.PartiallyChecked)
            else:
                e.setChecked(bool(v))
            e.clicked.connect(lambda checked: commit(name, bool(checked)))
            return e
        if kind == "enum":
            e = QComboBox()
            e.addItems(list(md.get("choices", ())))
            if mixed:
                e.insertItem(0, MIXED)
                e.setCurrentIndex(0)
            else:
                i = e.findText(str(v))
                if i < 0:
                    e.addItem(str(v))
                    i = e.count() - 1
                e.setCurrentIndex(i)
            e.activated.connect(lambda _i, e=e: e.currentText() != MIXED and commit(name, e.currentText()))
            return e
        if kind == "map":
            from ..content.scope import PRE_SEARING
            e = QComboBox()
            e.addItem("(none)", 0)
            for mid, label in sorted(PRE_SEARING.items(), key=lambda kv: kv[1]):
                e.addItem(f"{label} ({mid})", mid)
            if mixed:
                e.insertItem(0, MIXED, None)
                e.setCurrentIndex(0)
            else:
                i = e.findData(int(v))
                if i < 0:
                    e.addItem(f"map {v} (outside scope)", int(v))
                    i = e.count() - 1
                e.setCurrentIndex(i)
            e.activated.connect(lambda _i, e=e: e.currentData() is not None and commit(name, int(e.currentData())))
            return e
        if kind == "ref":
            return self._ref_editor(name, md, v)
        if kind == "list":
            return self._list_editor(name, md, v)
        if kind == "words":
            e = QLineEdit("" if mixed else " ".join(f"0x{int(x):04X}" for x in v))
            e.setFont(_mono())
            e.setPlaceholderText("0x328A 0xE3B9 …")

            def done_words(e=e):
                try:
                    words = [int(t, 0) for t in e.text().replace(",", " ").split()]
                    if any(not 0 <= w <= 0xFFFF for w in words):
                        raise ValueError
                    commit(name, words)
                except ValueError:
                    self.doc.message.emit(f"{name}: expected 16-bit words like 0x328A")
            e.editingFinished.connect(done_words)
            return e
        if kind in ("text", "lua"):
            e = QPlainTextEdit("" if mixed else str(v))
            e.setFixedHeight(70 if kind == "text" else 56)
            if kind == "lua":
                e.setFont(_mono())
                e.setPlaceholderText("Lua")
            filt = _FocusOut(lambda e=e: commit(name, e.toPlainText()))
            e.installEventFilter(filt)
            self._filters.append(filt)
            return e
        e = QLineEdit("" if mixed else str(v))
        if mixed:
            e.setPlaceholderText(MIXED)
        e.editingFinished.connect(lambda e=e: (e.text() or not mixed) and commit(name, e.text()))
        return e

    def _ref_ids(self, ref: str) -> list[str]:
        p = self.doc.profile
        m = self.doc.map
        if ref in ("route", "group"):
            if m is None:
                return []
            return [r.id for r in (m.routes if ref == "route" else m.groups)]
        try:
            return [r.id for r in p.collection(ref)]
        except KeyError:
            return []

    def _ref_editor(self, name: str, md: dict, v) -> QWidget:
        commit = self._committer()
        w = QWidget()
        h = QHBoxLayout(w)
        h.setContentsMargins(0, 0, 0, 0)
        e = QComboBox()
        e.setEditable(True)
        e.addItem("")
        e.addItems(sorted(self._ref_ids(md.get("ref", ""))))
        e.setCurrentText("" if v == MIXED else str(v))
        if v == MIXED:
            e.lineEdit().setPlaceholderText(MIXED)
        e.activated.connect(lambda _i: commit(name, e.currentText().strip()))
        e.lineEdit().editingFinished.connect(lambda: commit(name, e.currentText().strip()))
        h.addWidget(e, 1)
        ref = md.get("ref", "")
        if ref in ("agent", "dialogue", "quest", "script"):
            b = QToolButton()
            b.setText("…")
            b.setToolTip(f"Open the {ref}")

            def open_ref():
                rec = self.doc.profile.find(ref, e.currentText().strip())
                if rec is not None:
                    self.openRecord.emit(rec)
            b.clicked.connect(open_ref)
            h.addWidget(b)
        return w

    def _list_editor(self, name: str, md: dict, v) -> QWidget:
        commit = self._committer()
        choices = md.get("choices", ())
        current = [] if v == MIXED else list(v)
        if choices:
            b = QToolButton()
            b.setPopupMode(QToolButton.InstantPopup)
            b.setText(", ".join(current) or "(none)")
            menu = QMenu(b)
            for c in choices:
                act = menu.addAction(c)
                act.setCheckable(True)
                act.setChecked(c in current)

                def toggled(on, c=c):
                    new = [x for x in current if x != c] + ([c] if on else [])
                    commit(name, [x for x in choices if x in new])
                act.toggled.connect(toggled)
            b.setMenu(menu)
            return b
        e = QLineEdit(", ".join(current))
        e.setPlaceholderText("comma-separated ids")
        e.editingFinished.connect(lambda: commit(name, [x.strip() for x in e.text().split(",") if x.strip()]))
        return e
