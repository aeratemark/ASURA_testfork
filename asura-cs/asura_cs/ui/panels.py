"""Small panels: the log / validation list, and the preferences dialog."""

from __future__ import annotations

import time

from PySide6.QtCore import Qt, Signal
from PySide6.QtGui import QBrush, QColor
from PySide6.QtWidgets import (QCheckBox, QDialog, QDialogButtonBox, QDoubleSpinBox, QFileDialog, QFormLayout,
                               QHBoxLayout, QLineEdit, QListWidget, QPlainTextEdit, QPushButton, QTabWidget,
                               QTreeWidget, QTreeWidgetItem, QVBoxLayout, QWidget)


class LogPanel(QTabWidget):
    issueActivated = Signal(object)

    def __init__(self, parent=None):
        super().__init__(parent)
        self.log = QPlainTextEdit()
        self.log.setReadOnly(True)
        self.log.setMaximumBlockCount(5000)
        self.issues = QTreeWidget()
        self.issues.setHeaderLabels(["Level", "Where", "Message"])
        self.issues.setColumnWidth(0, 60)
        self.issues.setColumnWidth(1, 260)
        self.issues.itemDoubleClicked.connect(self._activate)
        self.addTab(self.log, "Log")
        self.addTab(self.issues, "Validation")

    def write(self, text: str) -> None:
        self.log.appendPlainText(time.strftime("%H:%M:%S  ") + text)

    def show_issues(self, issues) -> None:
        self.issues.clear()
        for i in issues:
            it = QTreeWidgetItem([i.level.upper(), i.where, i.message])
            it.setForeground(0, QBrush(QColor("#b00000" if i.level == "error" else "#806000")))
            it.setData(0, Qt.UserRole, i.obj)
            self.issues.addTopLevelItem(it)
        errors = sum(1 for i in issues if i.level == "error")
        self.setTabText(1, f"Validation ({errors} errors, {len(issues) - errors} warnings)")
        self.setCurrentIndex(1)

    def _activate(self, it, _col):
        obj = it.data(0, Qt.UserRole)
        if obj is not None:
            self.issueActivated.emit(obj)


class PreferencesDialog(QDialog):
    def __init__(self, settings, parent=None):
        super().__init__(parent)
        self.settings = settings
        self.setWindowTitle("Preferences")
        self.resize(620, 360)
        form = QFormLayout()
        self.asura = self._path_row(form, "ASURA checkout", settings.asura_root, dir_=True)
        self.dat = self._path_row(form, "Gw.dat (read-only)", settings.gw_dat, dir_=False)
        self.cache = self._path_row(form, "Cache folder", settings.cache_dir, dir_=True)
        self.gwmb = QListWidget()
        self.gwmb.addItems(settings.gwmb_dirs)
        self.gwmb.setToolTip("Folders containing GuildWarsMapBrowser 'Export full map' output (map_<id>.json).")
        row = QHBoxLayout()
        add = QPushButton("Add folder…")
        add.clicked.connect(self._add_gwmb)
        rem = QPushButton("Remove")
        rem.clicked.connect(lambda: [self.gwmb.takeItem(self.gwmb.row(i)) for i in self.gwmb.selectedItems()])
        row.addWidget(add)
        row.addWidget(rem)
        row.addStretch(1)
        box = QWidget()
        bl = QVBoxLayout(box)
        bl.setContentsMargins(0, 0, 0, 0)
        bl.addWidget(self.gwmb)
        bl.addLayout(row)
        form.addRow("GWMB exports", box)
        self.grid = QDoubleSpinBox()
        self.grid.setRange(1, 10000)
        self.grid.setValue(settings.grid_size)
        self.angle = QDoubleSpinBox()
        self.angle.setRange(0, 180)
        self.angle.setValue(settings.angle_snap)
        self.snap = QCheckBox("Snap to grid / angle")
        self.snap.setChecked(settings.snap)
        form.addRow("Grid size (units)", self.grid)
        form.addRow("Angle snap (°)", self.angle)
        form.addRow("", self.snap)
        buttons = QDialogButtonBox(QDialogButtonBox.Ok | QDialogButtonBox.Cancel)
        buttons.accepted.connect(self.accept)
        buttons.rejected.connect(self.reject)
        lay = QVBoxLayout(self)
        lay.addLayout(form)
        lay.addWidget(buttons)

    def _path_row(self, form, label, value, dir_: bool) -> QLineEdit:
        e = QLineEdit(value)
        b = QPushButton("…")
        b.setFixedWidth(28)

        def browse():
            if dir_:
                p = QFileDialog.getExistingDirectory(self, label, e.text())
            else:
                p, _ = QFileDialog.getOpenFileName(self, label, e.text(), "Gw.dat (*.dat);;All files (*)")
            if p:
                e.setText(p)
        b.clicked.connect(browse)
        w = QWidget()
        h = QHBoxLayout(w)
        h.setContentsMargins(0, 0, 0, 0)
        h.addWidget(e, 1)
        h.addWidget(b)
        form.addRow(label, w)
        return e

    def _add_gwmb(self):
        p = QFileDialog.getExistingDirectory(self, "GWMB export folder")
        if p:
            self.gwmb.addItem(p)

    def apply(self) -> None:
        s = self.settings
        s.asura_root = self.asura.text().strip()
        s.gw_dat = self.dat.text().strip()
        s.cache_dir = self.cache.text().strip() or s.cache_dir
        s.gwmb_dirs = [self.gwmb.item(i).text() for i in range(self.gwmb.count())]
        s.grid_size = float(self.grid.value())
        s.angle_snap = float(self.angle.value())
        s.snap = self.snap.isChecked()
        s.save()
