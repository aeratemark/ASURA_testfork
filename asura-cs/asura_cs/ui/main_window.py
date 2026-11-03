"""The editor window: 3D world view and Nox-style 2D view in the middle, docks around them."""

from __future__ import annotations

import os
from pathlib import Path

from PySide6.QtCore import QSettings, Qt
from PySide6.QtGui import QAction, QKeySequence
from PySide6.QtWidgets import (QApplication, QButtonGroup, QComboBox, QDockWidget, QFileDialog, QHBoxLayout,
                               QInputDialog, QLabel, QMainWindow, QMessageBox, QRadioButton, QSplitter, QToolBar,
                               QUndoView, QVBoxLayout, QWidget)

from .. import __version__, library
from ..content import store
from ..content.base_profile import build_base_profile, copy_profile
from ..content.model import AgentDef, Dialogue, MapObject, Quest, Route, Script, Group, slug
from ..content.scope import SCOPE_NAME, in_scope
from ..content.validate import validate
from ..export import EXPORTERS
from ..export import run as run_export
from ..gamedata import GameData, Settings
from .dialogue_editor import DialogueEditor
from .document import EditorDocument
from .hierarchy import HierarchyPanel
from .palette import PalettePanel
from .panels import LogPanel, PreferencesDialog
from .properties import PropertyPanel
from .quest_editor import QuestEditor
from .script_editor import ScriptEditor
from .viewport2d import FRONT, SIDE, TOP, MapView2D
from .viewport3d import Viewport3D


class WaypointTemplate(library.Template):
    """Arms waypoint placing for one route; each click appends the next waypoint."""

    def __init__(self, route_id: str):
        super().__init__(f"Route/{route_id}", f"waypoint of {route_id}", object_type="waypoint")
        self.route_id = route_id

    def instantiate(self, profile, map_id, x, y, z=0.0):
        m = profile.maps[map_id]
        self.defaults = {"route": self.route_id, "order": len(m.waypoints(self.route_id)),
                         "name": f"{self.route_id} #{len(m.waypoints(self.route_id))}",
                         "id": f"{self.route_id}_wp"}
        return super().instantiate(profile, map_id, x, y, z)


class MainWindow(QMainWindow):
    def __init__(self, settings: Settings):
        super().__init__()
        self.settings = settings
        self.gamedata = GameData(settings)
        self.doc = EditorDocument(self.gamedata)
        self.setWindowTitle("ASURA Construction Set")
        self.resize(1600, 1000)
        self.setDockNestingEnabled(True)

        # -- central: 3D | 2D --------------------------------------------------------------------
        self.v3d = Viewport3D(self.doc, settings)
        self.v2d = MapView2D(self.doc, settings, TOP)
        right = QWidget()
        rl = QVBoxLayout(right)
        rl.setContentsMargins(0, 0, 0, 0)
        rl.setSpacing(0)
        bar = QHBoxLayout()
        bar.setContentsMargins(4, 2, 4, 2)
        self.view_mode = QButtonGroup(self)
        for label, mode in (("Top", TOP), ("Front", FRONT), ("Side", SIDE)):
            rb = QRadioButton(label)
            rb.setChecked(mode == TOP)
            rb.toggled.connect(lambda on, m=mode: on and self.v2d.set_mode(m))
            self.view_mode.addButton(rb)
            bar.addWidget(rb)
        bar.addStretch(1)
        rl.addLayout(bar)
        rl.addWidget(self.v2d, 1)
        self.split = QSplitter(Qt.Horizontal)
        self.split.addWidget(self.v3d)
        self.split.addWidget(right)
        self.split.setSizes([900, 700])
        self.setCentralWidget(self.split)

        # -- docks -------------------------------------------------------------------------------------
        self.hierarchy = HierarchyPanel(self.doc)
        self.palette = PalettePanel(self.doc)
        self.props = PropertyPanel(self.doc)
        self.logp = LogPanel()
        self.dialogues = DialogueEditor(self.doc)
        self.scripts = ScriptEditor(self.doc)
        self.quests = QuestEditor(self.doc)
        self.undo_view = QUndoView(self.doc.undo_stack)
        self.d_hier = self._dock("World", self.hierarchy, Qt.LeftDockWidgetArea)
        self.d_pal = self._dock("Object Window", self.palette, Qt.LeftDockWidgetArea)
        self.d_props = self._dock("Properties", self.props, Qt.RightDockWidgetArea)
        self.d_undo = self._dock("History", self.undo_view, Qt.RightDockWidgetArea)
        self.d_log = self._dock("Log", self.logp, Qt.BottomDockWidgetArea)
        self.d_dlg = self._dock("Dialogue", self.dialogues, Qt.BottomDockWidgetArea)
        self.d_scr = self._dock("Scripts", self.scripts, Qt.BottomDockWidgetArea)
        self.d_qst = self._dock("Quests", self.quests, Qt.BottomDockWidgetArea)
        self.tabifyDockWidget(self.d_log, self.d_dlg)
        self.tabifyDockWidget(self.d_dlg, self.d_scr)
        self.tabifyDockWidget(self.d_scr, self.d_qst)
        self.tabifyDockWidget(self.d_props, self.d_undo)
        self.d_props.raise_()
        self.d_log.raise_()
        self.resizeDocks([self.d_hier, self.d_props], [330, 340], Qt.Horizontal)
        self.resizeDocks([self.d_log], [260], Qt.Vertical)

        # -- status bar ----------------------------------------------------------------------------
        self.st_cursor = QLabel("x —  y —  z —")
        self.st_sel = QLabel("0 selected")
        self.st_map = QLabel("")
        for w in (self.st_map, self.st_sel, self.st_cursor):
            self.statusBar().addPermanentWidget(w)

        self._build_actions()
        self._wire()
        self._restore_layout()

    # ------------------------------------------------------------------------------------------------
    def _dock(self, title, widget, area) -> QDockWidget:
        d = QDockWidget(title, self)
        d.setObjectName(f"dock_{title}")
        d.setWidget(widget)
        self.addDockWidget(area, d)
        return d

    def _act(self, menu, text, fn, shortcut=None, checkable=False, checked=False, tip=""):
        a = QAction(text, self)
        if shortcut:
            a.setShortcut(QKeySequence(shortcut))
        if checkable:
            a.setCheckable(True)
            a.setChecked(checked)
            a.toggled.connect(fn)
        else:
            a.triggered.connect(lambda _=False: fn())
        if tip:
            a.setStatusTip(tip)
        if menu is not None:
            menu.addAction(a)
        return a

    def _build_actions(self):
        mb = self.menuBar()
        f = mb.addMenu("&File")
        self.a_new = self._act(f, "New profile from base game…", self.new_from_base, "Ctrl+N",
                               tip="Copy ASURA's base content into a new self-contained profile (docs/34).")
        self._act(f, "New empty profile…", self.new_empty)
        self.a_open = self._act(f, "Open profile…", self.open_dialog, "Ctrl+O")
        self.a_save = self._act(f, "Save", self.save, "Ctrl+S")
        self._act(f, "Save copy as…", self.save_as, "Ctrl+Shift+S")
        f.addSeparator()
        self._act(f, "Apply spawns and portals to ASURA…", self.apply_to_asura,
                  tip="Write changed spawn points and portals to ASURA's database and restart the world servers.")
        ex = f.addMenu("Export")
        for key, (label, suffix, _fn) in EXPORTERS.items():
            self._act(ex, label + "…", lambda k=key, s=suffix: self.export(k, s))
        f.addSeparator()
        self._act(f, "Exit", self.close, "Alt+F4")

        e = mb.addMenu("&Edit")
        self.a_undo = self.doc.undo_stack.createUndoAction(self, "Undo")
        self.a_undo.setShortcut(QKeySequence.Undo)
        self.a_redo = self.doc.undo_stack.createRedoAction(self, "Redo")
        self.a_redo.setShortcuts([QKeySequence("Ctrl+Y"), QKeySequence(QKeySequence.Redo)])
        e.addAction(self.a_undo)
        e.addAction(self.a_redo)
        e.addSeparator()
        # World-editing shortcuts only fire while a view or the World tree has focus, so Del / Ctrl+A
        # keep their usual meaning in text fields, tables and the script editor.
        self.world_actions = [
            self._act(e, "Duplicate", self.doc.duplicate_selected, "Ctrl+D"),
            self._act(e, "Delete", self.doc.delete_selected, "Del"),
            self._act(e, "Select all in map",
                      lambda: self.doc.select(list(self.doc.map.objects) if self.doc.map else []), "Ctrl+A"),
        ]
        e.addSeparator()
        self._act(e, "Preferences…", self.preferences)

        w = mb.addMenu("&World")
        self._act(w, "Add map to profile…", self.add_map)
        self._act(w, f"Keep only {SCOPE_NAME} maps", self.restrict_to_scope)
        self.world_actions += [
            self._act(w, "Drop selection to ground", self.doc.drop_to_ground, "End"),
            self._act(w, "Rotate selection +15°", lambda: self.doc.rotate_selected(self.settings.angle_snap or 15), "]"),
            self._act(w, "Rotate selection −15°", lambda: self.doc.rotate_selected(-(self.settings.angle_snap or 15)),
                      "["),
        ]
        for a in self.world_actions:
            a.setShortcutContext(Qt.WidgetWithChildrenShortcut)
            self.split.addAction(a)
            self.hierarchy.addAction(a)
        self.a_snap = self._act(w, "Snap to grid", self._set_snap, "G", checkable=True, checked=self.settings.snap)
        w.addSeparator()
        self._act(w, "Validate profile", self.validate, "F7")
        self._act(w, "Reload game data", self.reload_gamedata, "F5")

        c = mb.addMenu("&Content")
        for kind in ("agent", "dialogue", "quest", "script"):
            self._act(c, f"New {kind}…", lambda k=kind: self.palette.new_record(k))
        c.addSeparator()
        self._act(c, "New route (then click waypoints)…", self.new_route)
        self._act(c, "New group…", self.new_group)
        self._act(c, "Assign selection to group…", self.assign_group)

        v = mb.addMenu("&View")
        layers = [("Terrain", "terrain", True), ("Path graph (walkable)", "navmesh", True),
                  ("Navigation graph", "navgraph", False), ("Scenery props", "props", True),
                  ("Labels", "labels", True), ("Grid", "grid", True)]
        for label, name, on in layers:
            self._act(v, label, lambda checked, n=name: self._layer(n, checked), checkable=True, checked=on)
        v.addSeparator()
        self._act(v, "Frame map (2D)", self.v2d.frame_all, "Home")
        v.addSeparator()
        for d in (self.d_hier, self.d_pal, self.d_props, self.d_undo, self.d_log, self.d_dlg, self.d_scr, self.d_qst):
            v.addAction(d.toggleViewAction())

        h = mb.addMenu("&Help")
        self._act(h, "Controls", self.show_controls, "F1")
        self._act(h, "About", self.about)

        tb = QToolBar("Main")
        tb.setObjectName("toolbar_main")
        self.addToolBar(tb)
        for a in (self.a_new, self.a_open, self.a_save):
            tb.addAction(a)
        tb.addSeparator()
        tb.addAction(self.a_undo)
        tb.addAction(self.a_redo)
        tb.addSeparator()
        tb.addWidget(QLabel(" Map: "))
        self.map_combo = QComboBox()
        self.map_combo.setMinimumWidth(260)
        self.map_combo.activated.connect(lambda _i: self.doc.set_map(self.map_combo.currentData()))
        tb.addWidget(self.map_combo)
        tb.addSeparator()
        tb.addAction(self.a_snap)
        self._act(tb, "Validate", self.validate)

    def _wire(self):
        d = self.doc
        d.message.connect(self.logp.write)
        d.message.connect(lambda t: self.statusBar().showMessage(t, 6000))
        d.profileReset.connect(self._refresh_maps)
        d.mapChanged.connect(self._map_changed)
        d.selectionChanged.connect(lambda s: self.st_sel.setText(f"{len(s)} selected"))
        d.contentChanged.connect(self._content_changed)
        d.undo_stack.cleanChanged.connect(lambda _c: self._title())
        d.armedChanged.connect(lambda _t: (self.v2d.viewport().update(), self.v3d.update()))
        self.v2d.cursorMoved.connect(self._cursor)
        self.v3d.cursorMoved.connect(self._cursor)
        self.v2d.focusRequested.connect(self.focus_object)
        self.hierarchy.focusRequested.connect(self.focus_object)
        for src in (self.palette, self.props):
            src.openRecord.connect(self.open_record)
        self.logp.issueActivated.connect(self._issue)

    def _restore_layout(self):
        qs = QSettings("ASURA", "ConstructionSet")
        geo, state = qs.value("geometry"), qs.value("state")
        if geo is not None:
            self.restoreGeometry(geo)
        if state is not None:
            self.restoreState(state)

    # -- startup ---------------------------------------------------------------------------------------
    def startup(self):
        s = self.settings
        if not s.asura_root:
            guess = os.environ.get("ASURA_ROOT") or ""
            for cand in (guess, Path.cwd(), Path.cwd().parent, Path.cwd().parent.parent):
                if cand and (Path(cand) / "crates" / "asura-world").is_dir():
                    s.asura_root = str(cand)
                    s.save()
                    break
        if not s.asura_root:
            QMessageBox.information(self, "ASURA Construction Set",
                                    "Point the editor at your ASURA checkout (and optionally Gw.dat and GWMB "
                                    "exports) to begin.")
            self.preferences()
        self.reload_gamedata(reset_view=False)
        if s.last_profile and store.is_profile(s.last_profile):
            self.open_profile(Path(s.last_profile))
        elif self.gamedata.catalogue.maps:
            self.logp.write("No profile open. File > New profile from base game… copies ASURA's maps, spawns "
                            "and portals into a profile you can edit.")

    def reload_gamedata(self, reset_view: bool = True):
        for note in self.gamedata.load_asura():
            self.logp.write(note)
        if self.settings.gw_dat:
            self.logp.write(f"Gw.dat: {self.settings.gw_dat} (read-only; decoded entries cached in "
                            f"{self.settings.cache_dir})")
        if reset_view and self.doc.map_id is not None:
            mid = self.doc.map_id
            self.doc.map_id = None
            self.doc.set_map(mid)

    # -- profile files ------------------------------------------------------------------------------------
    def _confirm_discard(self) -> bool:
        self.scripts.apply()
        if not self.doc.dirty:
            return True
        r = QMessageBox.question(self, "Unsaved changes", "Save changes to the current profile?",
                                 QMessageBox.Save | QMessageBox.Discard | QMessageBox.Cancel)
        if r == QMessageBox.Save:
            return self.save()
        return r == QMessageBox.Discard

    def new_from_base(self):
        if not self._confirm_discard():
            return
        if not self.gamedata.catalogue.maps:
            QMessageBox.warning(self, "New profile", "No ASURA catalogue loaded (Edit > Preferences).")
            return
        name, ok = QInputDialog.getText(self, "New profile", "Profile name:", text="My Ascalon")
        if not ok or not name.strip():
            return
        default_parent = Path(self.settings.asura_root or Path.home()) / "content" / "profiles"
        folder = QFileDialog.getExistingDirectory(self, "Where to create the profile folder", str(default_parent))
        if not folder:
            return
        target = Path(folder) / slug(name)
        if target.exists() and any(target.iterdir()):
            QMessageBox.warning(self, "New profile", f"{target} is not empty.")
            return
        base = build_base_profile(self.gamedata.catalogue)
        prof = copy_profile(base, slug(name), name.strip())
        store.save(prof, target)
        self.open_profile(target)
        self.logp.write(f"Created profile {prof.name!r} from the base game: {len(prof.maps)} maps.")

    def new_empty(self):
        if not self._confirm_discard():
            return
        name, ok = QInputDialog.getText(self, "New empty profile", "Profile name:", text="Custom")
        if not ok or not name.strip():
            return
        folder = QFileDialog.getExistingDirectory(self, "Where to create the profile folder")
        if not folder:
            return
        from ..content.model import Profile
        target = Path(folder) / slug(name)
        store.save(Profile(id=slug(name), name=name.strip()), target)
        self.open_profile(target)
        self.logp.write("Empty profile created. World > Add map to profile… to start.")

    def open_dialog(self):
        if not self._confirm_discard():
            return
        folder = QFileDialog.getExistingDirectory(self, "Open profile folder", self.settings.last_profile or "")
        if folder:
            if not store.is_profile(folder):
                QMessageBox.warning(self, "Open profile", "That folder has no profile.json.")
                return
            self.open_profile(Path(folder))

    def open_profile(self, folder: Path):
        try:
            prof = store.load(folder)
        except Exception as e:
            QMessageBox.critical(self, "Open profile", f"Could not load {folder}:\n{e}")
            return
        self.doc.set_profile(prof, folder)
        self.palette.refresh()
        self.settings.last_profile = str(folder)
        self.settings.save()
        outside = [mid for mid in prof.maps if not in_scope(mid)]
        if outside and QMessageBox.question(
                self, "Pre-Searing only",
                f"This profile has {len(outside)} maps outside {SCOPE_NAME}. Remove them? "
                "(Undoable; nothing is deleted from disk until you save.)") == QMessageBox.Yes:
            self.restrict_to_scope(ask=False)
        self.logp.write(f"Opened profile {prof.name!r} ({folder}): {len(prof.maps)} maps, {len(prof.agents)} "
                        f"agents, {len(prof.dialogues)} dialogues, {len(prof.quests)} quests, "
                        f"{len(prof.scripts)} scripts.")
        self._title()

    def save(self) -> bool:
        self.scripts.apply()
        if self.doc.folder is None:
            return self.save_as()
        try:
            self.doc.save()
        except Exception as e:
            QMessageBox.critical(self, "Save", f"Save failed:\n{e}")
            return False
        self._title()
        return True

    def save_as(self) -> bool:
        self.scripts.apply()
        folder = QFileDialog.getExistingDirectory(self, "Save profile copy into folder")
        if not folder:
            return False
        name, ok = QInputDialog.getText(self, "Save copy", "Profile name:", text=self.doc.profile.name + " copy")
        if not ok:
            return False
        target = Path(folder) / slug(name)
        prof = copy_profile(self.doc.profile, slug(name), name)
        store.save(prof, target)
        self.open_profile(target)
        return True

    def export(self, key: str, suffix: str):
        self.scripts.apply()
        issues = validate(self.doc.profile, self.gamedata.path_graphs)
        self.logp.show_issues(issues)
        errors = [i for i in issues if i.level == "error"]
        if errors and QMessageBox.question(self, "Export", f"{len(errors)} validation errors. Export anyway?") \
                != QMessageBox.Yes:
            return
        out, _ = QFileDialog.getSaveFileName(self, "Export", f"{self.doc.profile.id}{suffix}", f"*{suffix}")
        if not out:
            return
        written = run_export(key, self.doc.profile, Path(out))
        self.logp.write(f"Exported {EXPORTERS[key][0]}: " + ", ".join(str(p) for p in written))

    def apply_to_asura(self):
        from .deploy_window import DeployWindow
        self.scripts.apply()
        DeployWindow(self.settings, self.doc.profile, str(self.doc.folder or ""), self).exec()

    def closeEvent(self, ev):
        if not self._confirm_discard():
            ev.ignore()
            return
        qs = QSettings("ASURA", "ConstructionSet")
        qs.setValue("geometry", self.saveGeometry())
        qs.setValue("state", self.saveState())
        ev.accept()

    # -- world / content ---------------------------------------------------------------------------------
    def add_map(self):
        cat = self.gamedata.catalogue
        choices = [m for m in cat.sorted()
                   if in_scope(m.map_id) and m.map_id not in self.doc.profile.maps and m.map_file_id]
        if not choices:
            QMessageBox.information(self, "Add map", f"Every {SCOPE_NAME} map is already in the profile.")
            return
        labels = [f"{m.map_id}: {m.name or '(unnamed)'} [{m.kind}]" for m in choices]
        pick, ok = QInputDialog.getItem(self, "Add map", "Map:", labels, 0, False)
        if ok:
            m = choices[labels.index(pick)]
            self.doc.ensure_map(m.map_id)
            self._refresh_maps()
            self.doc.set_map(m.map_id)

    def restrict_to_scope(self, ask: bool = True):
        maps = self.doc.profile.maps
        outside = sorted(mid for mid in maps if not in_scope(mid))
        if not outside:
            self.logp.write(f"The profile already contains only {SCOPE_NAME} maps.")
            return
        if ask and QMessageBox.question(self, "Pre-Searing only",
                                        f"Remove {len(outside)} maps outside {SCOPE_NAME}?") != QMessageBox.Yes:
            return
        from ..content import commands as C
        removed = {mid: maps[mid] for mid in outside}

        class _Restrict(C.Command):
            text = f"Keep only {SCOPE_NAME} maps"

            def redo(self, hub):
                for mid in removed:
                    maps.pop(mid, None)
                hub.emit("removed", next(iter(removed.values())), maps)

            def undo(self, hub):
                maps.update(removed)
                hub.emit("added", next(iter(removed.values())), maps)

        if self.doc.map_id in removed:
            self.doc.set_map(148 if 148 in maps else next((m for m in sorted(maps) if in_scope(m)), None))
        self.doc.push(_Restrict())
        self._refresh_maps()
        self.logp.write(f"Removed {len(outside)} maps outside {SCOPE_NAME}; {len(maps)} remain. Save to make it stick.")

    def new_route(self):
        m = self.doc.map
        if m is None:
            return
        rid, ok = QInputDialog.getText(self, "New route", "Route id:", text=self.doc.profile.unique_id("route"))
        if not ok or not rid:
            return
        rid = self.doc.profile.unique_id(rid)
        from ..content import commands as C
        r = Route(id=rid, name=rid)
        self.doc.push(C.AddRecords([(m.routes, r)], f"New route {rid}"))
        self.doc.arm(WaypointTemplate(rid))
        self.logp.write(f"Route {rid}: click in the Top or 3D view to add waypoints in order; Esc to finish.")

    def new_group(self):
        m = self.doc.map
        if m is None:
            return
        gid, ok = QInputDialog.getText(self, "New group", "Group id:", text=self.doc.profile.unique_id("group"))
        if ok and gid:
            from ..content import commands as C
            g = Group(id=self.doc.profile.unique_id(gid), name=gid)
            self.doc.push(C.AddRecords([(m.groups, g)], f"New group {g.id}"))

    def assign_group(self):
        m = self.doc.map
        agents = [o for o in self.doc.selected_objects() if hasattr(o, "group_id")]
        if m is None or not agents or not m.groups:
            QMessageBox.information(self, "Group", "Select agents, and create a group first.")
            return
        gid, ok = QInputDialog.getItem(self, "Assign group", "Group:", [g.id for g in m.groups], 0, False)
        if ok:
            self.doc.set_fields(agents, {"group_id": gid}, "Assign group")

    def validate(self):
        self.scripts.apply()
        issues = validate(self.doc.profile, self.gamedata.path_graphs)
        self.logp.show_issues(issues)
        self.d_log.raise_()
        self.logp.write(f"Validation: {sum(i.level == 'error' for i in issues)} errors, "
                        f"{sum(i.level == 'warning' for i in issues)} warnings.")

    def _issue(self, obj):
        if isinstance(obj, MapObject):
            m = self.doc.profile.map_of(obj)
            if m is not None and m.map_id != self.doc.map_id:
                self.doc.set_map(m.map_id)
                self._sync_combo()
            self.focus_object(obj)
        else:
            self.open_record(obj)

    def open_record(self, rec):
        if rec is None:
            return
        if isinstance(rec, Dialogue):
            self.dialogues.open(rec)
            self.d_dlg.raise_()
        elif isinstance(rec, Script):
            self.scripts.open(rec)
            self.d_scr.raise_()
        elif isinstance(rec, Quest):
            self.quests.open(rec)
            self.d_qst.raise_()
            self.doc.select([rec])
        elif isinstance(rec, AgentDef):
            self.doc.select([rec])
            self.d_props.raise_()

    def focus_object(self, obj):
        if isinstance(obj, MapObject):
            self.doc.select([obj])
            self.v3d.focus_objects([obj])
            self.v2d.frame_objects([obj])

    # -- view helpers -------------------------------------------------------------------------------------
    def _layer(self, name, on):
        self.v2d.set_layer(name, on)
        if name in ("navmesh", "props", "labels"):
            self.v3d.set_layer(name, on)

    def _set_snap(self, on):
        self.settings.snap = on
        self.settings.save()

    def _refresh_maps(self):
        self.map_combo.clear()
        for mid, m in sorted(self.doc.profile.maps.items()):
            if in_scope(mid):
                self.map_combo.addItem(m.label(), mid)
        self._sync_combo()

    def _sync_combo(self):
        i = self.map_combo.findData(self.doc.map_id)
        if i >= 0:
            self.map_combo.setCurrentIndex(i)

    def _map_changed(self, mid):
        self._sync_combo()
        m = self.doc.map
        sc = self.doc.scene()
        if m is not None:
            src = []
            if sc and sc.heightmap is not None:
                src.append("terrain")
            if sc and sc.path_graph is not None:
                src.append("path graph")
            if sc and sc.gwmb is not None:
                src.append("GWMB models")
            self.st_map.setText(f"{m.label()} · {', '.join(src) or 'no geometry source'}")
        self.v2d.frame_all()

    def _content_changed(self, kind, obj):
        if hasattr(obj, "map_id") and kind in ("added", "removed"):
            self._refresh_maps()

    def _cursor(self, x, y, z):
        self.st_cursor.setText(f"x {x:9.1f}   y {y:9.1f}   z {z:8.1f}")

    def _title(self):
        p = self.doc.profile
        star = "*" if self.doc.dirty else ""
        self.setWindowTitle(f"{star}{p.name} — ASURA Construction Set")

    def preferences(self):
        dlg = PreferencesDialog(self.settings, self)
        if dlg.exec():
            dlg.apply()
            self.a_snap.setChecked(self.settings.snap)
            self.gamedata._dat = None
            self.reload_gamedata()

    def show_controls(self):
        QMessageBox.information(self, "Controls", (
            "3D VIEW\n"
            "  LMB: select / drag on terrain / place armed item    Ctrl/Shift+LMB: add/toggle\n"
            "  RMB drag: orbit    MMB drag: pan    Wheel: zoom    W A S D Q E: fly\n"
            "  F: focus selection    Z / X: rotate selection    Double-click: focus\n\n"
            "2D VIEW (Top / Front / Side)\n"
            "  LMB: select, drag to move (snaps with G), rubber-band select on empty ground\n"
            "  Alt+drag: rotate (Top)    MMB or Space+LMB: pan    Wheel: zoom    F: frame\n"
            "  RMB: context menu / stop placing\n\n"
            "OBJECT WINDOW\n"
            "  Click a template to arm it, then click in a view; or drag it into a view.\n"
            "  Drag an agent definition from Records to place it.\n\n"
            "Del delete · Ctrl+D duplicate · End drop to ground · [ ] rotate · F7 validate · Ctrl+Z / Ctrl+Y"))

    def about(self):
        QMessageBox.about(self, "About", (
            f"<b>ASURA Construction Set</b> {__version__}<br>"
            "A level and content editor for the ASURA Guild Wars server.<br><br>"
            "Map geometry formats follow <a href='https://github.com/Jonathan-Greve/GuildWarsMapBrowser'>"
            "Guild Wars Map Browser</a> by Jonathan Bjørn Greve.<br>"
            "Gw.dat index and decompressor ported from ASURA.<br>"
            "Client field notes (NPC definitions, dialog option kinds, text limits) from the "
            "<a href='https://github.com/GameJawnsInc/Rurik'>Rurik</a> research project.<br><br>"
            "Game data is read, never written. No ArenaNet data ships with this tool."))


def run_app(argv) -> int:
    from . import style
    app = QApplication(argv)
    app.setApplicationName("ASURA Construction Set")
    style.apply(app)
    settings = Settings.load()
    win = MainWindow(settings)
    win.show()
    win.startup()
    if len(argv) > 1 and store.is_profile(argv[1]):
        win.open_profile(Path(argv[1]))
    return app.exec()


__all__ = ["MainWindow", "run_app"]


def main_entry() -> int:
    import sys
    return run_app(sys.argv)
