"""Apply to ASURA: compare the profile's spawn points with the database, apply the changed ones with
psql, and restart the world servers through the running supervisor. Long steps run off the UI thread."""

from __future__ import annotations

import sys
import threading
from pathlib import Path

from PySide6.QtCore import QObject, Qt, Signal
from PySide6.QtGui import QBrush, QColor, QFont
from PySide6.QtWidgets import (QApplication, QCheckBox, QDialog, QFileDialog, QFormLayout, QGroupBox, QHBoxLayout,
                               QLabel, QLineEdit, QMessageBox, QPlainTextEdit, QPushButton, QTreeWidget,
                               QTreeWidgetItem, QVBoxLayout, QWidget)

from .. import deploy
from ..content import store
from ..gamedata import Settings

WARNING = ("Note: <b>asura-supervisor start</b> re-seeds <i>tools/map-config.sql</i>, which puts the stock "
           "spawn back for the catalogue maps (Ascalon City among them). After a full start, apply again. "
           "Restarting a world from here does not re-seed.")


class _Bridge(QObject):
    line = Signal(str)
    done = Signal(object)   # payload from the worker, or an Exception


class DeployWindow(QDialog):
    def __init__(self, settings: Settings, profile=None, profile_folder: str = "", parent=None):
        super().__init__(parent)
        self.settings = settings
        self.profile = profile
        self.rows: list[deploy.SpawnRow] = []
        self.busy = False
        self.bridge = _Bridge()
        self.bridge.line.connect(self._log)
        self.setWindowTitle("Apply spawns and portals to ASURA")
        self.resize(820, 680)

        form = QFormLayout()
        self.asura = self._path(form, "ASURA checkout", settings.asura_root, True)
        self.prof = self._path(form, "Profile folder", profile_folder or settings.last_profile, True)
        self.url_label = QLabel()
        self.psql_label = QLabel()
        form.addRow("Database", self.url_label)
        form.addRow("psql", self.psql_label)

        self.tree = QTreeWidget()
        self.tree.setHeaderLabels(["Map", "Profile spawn", "Database now", "Status"])
        self.tree.setColumnWidth(0, 260)
        self.tree.setColumnWidth(1, 170)
        self.tree.setColumnWidth(2, 170)
        maps_box = QGroupBox("Spawn points and travel portals (index-0 spawn is the default arrival)")
        ml = QVBoxLayout(maps_box)
        ml.addWidget(self.tree)
        row = QHBoxLayout()
        self.only_changed = QCheckBox("Show only changed maps")
        self.only_changed.setChecked(True)
        self.only_changed.toggled.connect(self._fill_tree)
        refresh = QPushButton("Compare with database")
        refresh.clicked.connect(self.compare)
        row.addWidget(self.only_changed)
        row.addStretch(1)
        row.addWidget(refresh)
        ml.addLayout(row)

        svc_box = QGroupBox("Restart afterwards (needs the supervisor running)")
        self.svc_layout = QHBoxLayout(svc_box)
        self.svc_checks: list[QCheckBox] = []

        warn = QLabel(WARNING)
        warn.setWordWrap(True)
        warn.setStyleSheet("color: #604000;")

        buttons = QHBoxLayout()
        self.preview_btn = QPushButton("Preview SQL")
        self.preview_btn.clicked.connect(self.preview)
        self.apply_btn = QPushButton("Apply && restart")
        self.apply_btn.setDefault(True)
        self.apply_btn.clicked.connect(self.apply)
        sql_btn = QPushButton("Apply SQL file…")
        sql_btn.setToolTip("Apply a file from File › Export › ASURA SQL instead of the table above.")
        sql_btn.clicked.connect(self.apply_file)
        buttons.addWidget(sql_btn)
        status_btn = QPushButton("Supervisor status")
        status_btn.clicked.connect(self.supervisor_status)
        close = QPushButton("Close")
        close.clicked.connect(self.close)
        for b in (self.preview_btn, status_btn):
            buttons.addWidget(b)
        buttons.addStretch(1)
        buttons.addWidget(self.apply_btn)
        buttons.addWidget(close)

        self.out = QPlainTextEdit()
        self.out.setReadOnly(True)
        f = QFont("Consolas")
        f.setStyleHint(QFont.Monospace)
        self.out.setFont(f)

        lay = QVBoxLayout(self)
        lay.addLayout(form)
        lay.addWidget(maps_box, 3)
        lay.addWidget(svc_box)
        lay.addWidget(warn)
        lay.addLayout(buttons)
        lay.addWidget(self.out, 2)

        # A profile handed over by the editor is used as it is in memory (saved or not).
        self._loaded = profile_folder if profile is not None else ""
        self._svc_note = None
        self.asura.editingFinished.connect(self.reload)
        self.prof.editingFinished.connect(self.reload)
        self.reload()

    # -- helpers -----------------------------------------------------------------------------------
    def _path(self, form, label, value, is_dir) -> QLineEdit:
        e = QLineEdit(value or "")
        b = QPushButton("…")
        b.setFixedWidth(28)

        def browse():
            p = QFileDialog.getExistingDirectory(self, label, e.text())
            if p:
                e.setText(p)
                self.reload()
        b.clicked.connect(browse)
        w = QWidget()
        h = QHBoxLayout(w)
        h.setContentsMargins(0, 0, 0, 0)
        h.addWidget(e, 1)
        h.addWidget(b)
        form.addRow(label, w)
        return e

    def _log(self, text: str) -> None:
        self.out.appendPlainText(text.rstrip())

    def _background(self, work, on_done) -> None:
        """Run ``work(log)`` on a thread; ``on_done(result)`` back on the UI thread."""
        if self.busy:
            return
        self.busy = True
        self.apply_btn.setEnabled(False)
        QApplication.setOverrideCursor(Qt.BusyCursor)

        def finished(result):
            self.bridge.done.disconnect(finished)
            self.busy = False
            self.apply_btn.setEnabled(True)
            QApplication.restoreOverrideCursor()
            on_done(result)

        self.bridge.done.connect(finished)

        def runner():
            try:
                result = work(self.bridge.line.emit)
            except Exception as e:  # reported in the log, never swallowed
                result = e
            self.bridge.done.emit(result)

        threading.Thread(target=runner, daemon=True).start()

    # -- state ------------------------------------------------------------------------------------
    def reload(self) -> None:
        root = self.asura.text().strip()
        self.url = deploy.database_url(root) if root else None
        self.psql = deploy.find_psql()
        self.url_label.setText(deploy.mask_url(self.url) if self.url else
                               "<span style='color:#a00'>ASURA_DATABASE_URL not found in .env</span>")
        self.psql_label.setText(self.psql or "<span style='color:#a00'>psql not found (install PostgreSQL "
                                             "or add its bin folder to PATH)</span>")
        folder = self.prof.text().strip()
        if folder and store.is_profile(folder) and (self.profile is None or folder != getattr(self, "_loaded", "")):
            try:
                self.profile = store.load(folder)
                self._loaded = folder
            except Exception as e:
                self._log(f"Could not load profile: {e}")
        for c in self.svc_checks + ([self._svc_note] if self._svc_note else []):
            c.setParent(None)
        self.svc_checks, self._svc_note = [], None
        for s in deploy.services(root) if root else []:
            if s.is_world:
                c = QCheckBox(s.name)
                c.setChecked(True)
                self.svc_layout.addWidget(c)
                self.svc_checks.append(c)
        if not self.svc_checks:
            self._svc_note = QLabel("no world services in config/services.toml")
            self.svc_layout.addWidget(self._svc_note)
        self.compare()

    def _note(self, text: str) -> None:
        it = QTreeWidgetItem([text])
        it.setFirstColumnSpanned(True)
        it.setFlags(Qt.ItemIsEnabled)
        it.setForeground(0, QBrush(QColor("#806000")))
        self.tree.addTopLevelItem(it)

    def compare(self) -> None:
        if self.profile is None:
            self.tree.clear()
            self._note("No profile loaded: choose the profile folder (the one containing profile.json).")
            return
        if not (self.url and self.psql):
            self.rows = deploy.spawn_rows(self.profile, {})
            self._fill_tree()
            return
        url, psql, prof = self.url, self.psql, self.profile

        def work(log):
            portals = deploy.current_portals(psql, url)
            if portals is None:
                log("map_portal does not exist yet: run the schema step (run_server.py Setup, or "
                    "asura-supervisor schema) so the world can read portals. Applying creates it too.")
            return deploy.spawn_rows(prof, deploy.current_spawns(psql, url), portals)

        def done(result):
            if isinstance(result, Exception):
                self._log(f"Could not read map_config: {result}")
                self.rows = deploy.spawn_rows(prof, {})
            else:
                self.rows = result
                n = sum(r.changed for r in self.rows)
                self._log(f"Compared with the database: {n} map(s) differ.")
            self._fill_tree()
        self._background(work, done)

    def _fill_tree(self) -> None:
        self.tree.clear()
        fmt = lambda s: "—" if s is None else f"({s[0]:.1f}, {s[1]:.1f}) p{s[2]}"  # noqa: E731
        for r in self.rows:
            if self.only_changed.isChecked() and not r.changed:
                continue
            if r.current is None:
                status = "not in database (skipped)"
            elif not r.changed:
                status = "same"
            else:
                status = " + ".join(x for x, on in (("spawn changed", r.spawn_changed),
                                                    (f"portals changed ({r.portal_count})", r.portals_changed))
                                    if on)
            it = QTreeWidgetItem([f"{r.map_id}  {r.name}", fmt(r.new) + (f" +{r.count - 1}" if r.count > 1 else ""),
                                  fmt(r.current), status])
            it.setData(0, Qt.UserRole, r.map_id)
            if r.current is not None:
                it.setCheckState(0, Qt.Checked if r.changed else Qt.Unchecked)
            self.tree.addTopLevelItem(it)
        if self.tree.topLevelItemCount() == 0:
            if not self.rows:
                self._note("The profile has no maps.")
            else:
                self._note(f"None of the {len(self.rows)} maps has a spawn point that differs from the database. "
                           "Untick 'Show only changed maps' to see them all.")
                self._note("Only spawn points are applied: NPCs, dialogue and quests can't go into ASURA yet. "
                           "Moved the spawn in the editor? Save the profile (Ctrl+S) first.")

    def selected_maps(self) -> list[int]:
        out = []
        for i in range(self.tree.topLevelItemCount()):
            it = self.tree.topLevelItem(i)
            if it.checkState(0) == Qt.Checked:
                out.append(int(it.data(0, Qt.UserRole)))
        return out

    # -- actions ----------------------------------------------------------------------------------
    def preview(self) -> None:
        maps = self.selected_maps()
        if self.profile is None or not maps:
            self._log("Nothing selected.")
            return
        self._log(deploy.sql_for_maps(self.profile, maps))

    def supervisor_status(self) -> None:
        root = self.asura.text().strip()

        def work(log):
            log("$ " + " ".join(deploy.supervisor_command(root) + ["status"]))
            return deploy.status(root)

        self._background(work, lambda r: self._log(str(r) if isinstance(r, Exception) else (r.stdout + r.stderr)))

    def apply_file(self) -> None:
        path, _ = QFileDialog.getOpenFileName(self, "SQL file to apply", self.prof.text(), "SQL (*.sql)")
        if not path:
            return
        sql = Path(path).read_text(encoding="utf-8")
        if "map_spawn" not in sql and "map_config" not in sql:
            QMessageBox.warning(self, "Apply SQL file", "That file doesn't look like a spawn-point export.")
            return
        self._run_apply(sql, f"the spawn points in {Path(path).name}")

    def apply(self) -> None:
        maps = self.selected_maps()
        if self.profile is None or not maps:
            QMessageBox.information(self, "Apply", "Tick the maps to apply (only changed ones are ticked).")
            return
        if not (self.url and self.psql):
            QMessageBox.warning(self, "Apply", "Need ASURA_DATABASE_URL (in ASURA's .env) and psql.")
            return
        self._run_apply(deploy.sql_for_maps(self.profile, maps), f"spawn points of {len(maps)} map(s)",
                        ", ".join(map(str, maps)))

    def _run_apply(self, sql: str, what: str, label: str = "") -> None:
        root = self.asura.text().strip()
        if not (self.url and self.psql):
            QMessageBox.warning(self, "Apply", "Need ASURA_DATABASE_URL (in ASURA's .env) and psql.")
            return
        worlds = [c.text() for c in self.svc_checks if c.isChecked()]
        if QMessageBox.question(self, "Apply", f"Write {what} to the database"
                                + (f" and restart {', '.join(worlds)}?" if worlds else "?")) != QMessageBox.Yes:
            return
        url, psql, prof = self.url, self.psql, self.profile
        maps = label

        def work(log):
            log(f"$ psql -v ON_ERROR_STOP=1 -q -f <spawns.sql> {deploy.mask_url(url)}")
            r = deploy.apply_sql(psql, url, sql)
            if r.returncode != 0:
                log(r.stderr or r.stdout)
                return "psql failed; nothing was changed (the SQL runs in one transaction)."
            log(f"Applied {what}" + (f" (maps {maps})." if maps else "."))
            for w in worlds:
                log("$ " + " ".join(deploy.supervisor_command(root) + ["restart", w]))
                rr = deploy.restart(root, w)
                log((rr.stdout + rr.stderr).strip() or f"exit {rr.returncode}")
                if rr.returncode != 0:
                    log(f"Restart of {w} failed - is the supervisor running? (asura-supervisor start). "
                        "If you start it now, apply again afterwards: start re-seeds the catalogue spawns.")
            if prof is None:
                return "Done."
            return deploy.spawn_rows(prof, deploy.current_spawns(psql, url), deploy.current_portals(psql, url))

        def done(result):
            if isinstance(result, Exception):
                self._log(f"Error: {result}")
            elif isinstance(result, str):
                self._log(result)
            else:
                self.rows = result
                self._fill_tree()
                self._log("Done. Rejoin the map (or travel) in the client to try the new spawns and portals.")
        self._background(work, done)


def main() -> int:
    from . import style
    app = QApplication(sys.argv)
    style.apply(app)
    settings = Settings.load()
    folder = sys.argv[1] if len(sys.argv) > 1 else settings.last_profile
    w = DeployWindow(settings, profile_folder=folder)
    w.show()
    return app.exec()


if __name__ == "__main__":
    sys.exit(main())


__all__ = ["DeployWindow", "main", "Path"]
