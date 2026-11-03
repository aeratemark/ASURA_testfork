"""The 3D world view: Panda3D rendering offscreen, shown inside a Qt widget.

Panda3D draws into an offscreen buffer whose colour is copied to RAM each frame and painted by Qt.
That keeps all input in Qt (one event model for every viewport) and works the same on Windows and
Linux. Frames are rendered only when something changed.

Controls (the window must have focus):
  LMB             select / drag the selection along the terrain / place the armed palette item
  Ctrl/Shift+LMB  add to / toggle selection
  RMB drag        orbit           MMB drag   pan          wheel   zoom
  W A S D / Q E   fly             F          focus selection
  Z / X           rotate selection -/+ angle snap         End      drop selection to ground
"""

from __future__ import annotations

import math

import numpy as np
from PySide6.QtCore import QPointF, QTimer, Qt, Signal
from PySide6.QtGui import QColor, QImage, QPainter
from PySide6.QtWidgets import QLabel, QVBoxLayout, QWidget

from ..content.model import AgentPlacement, Area, MapObject, TravelPortal, Waypoint
from ..world import geometry as G
from .viewport2d import MIME_AGENT, MIME_TEMPLATE

BUFFER_SIZE = (1600, 1000)

try:  # Panda3D is optional: without it the editor still runs with the 2D views.
    import panda3d  # noqa: F401
    HAVE_PANDA = True
except ImportError:  # pragma: no cover
    HAVE_PANDA = False


# ----------------------------------------------------------------------------------------------
class PandaWorld:
    """Owns the single ShowBase and the scene graph. Not a widget."""

    _instance = None

    @classmethod
    def get(cls) -> "PandaWorld":
        if cls._instance is None:
            cls._instance = cls()
        return cls._instance

    def __init__(self):
        from panda3d.core import (AmbientLight, DirectionalLight, GraphicsOutput, Texture, Vec4,
                                  loadPrcFileData)
        loadPrcFileData("", f"window-type offscreen\nwin-size {BUFFER_SIZE[0]} {BUFFER_SIZE[1]}\n"
                            "audio-library-name null\nsync-video false\nload-display pandagl\n"
                            "notify-level-display error\n")
        from direct.showbase.ShowBase import ShowBase
        self.base = ShowBase(windowType="offscreen")
        self.base.disableMouse()
        self.base.setBackgroundColor(0.58, 0.68, 0.80, 1)
        self.tex = Texture("capture")
        self.base.win.addRenderTexture(self.tex, GraphicsOutput.RTMCopyRam, GraphicsOutput.RTPColor)
        render = self.base.render
        amb = AmbientLight("ambient")
        amb.setColor(Vec4(0.45, 0.45, 0.5, 1))
        sun = DirectionalLight("sun")
        sun.setColor(Vec4(0.75, 0.72, 0.65, 1))
        self.sun_np = render.attachNewNode(sun)
        self.sun_np.setHpr(30, -55, 0)
        render.setLight(render.attachNewNode(amb))
        render.setLight(self.sun_np)
        lens = self.base.cam.node().getLens()
        lens.setNearFar(10.0, 400000.0)
        self.lens = lens
        self.world_root = render.attachNewNode("world")       # terrain, props, navmesh (read-only)
        self.objects_root = render.attachNewNode("objects")   # profile content
        self.overlay_root = render.attachNewNode("overlay")   # rings, routes
        self.overlay_root.setLightOff()
        self.pawn = make_geom_node("pawn", *G.pawn_mesh())

    def render_image(self, cam: G.OrbitCamera) -> QImage | None:
        from panda3d.core import Point3, Vec3
        self.lens.setFov(cam.hfov, cam.vfov)
        pos, tgt = cam.position(), cam.target
        self.base.camera.setPos(Point3(*map(float, pos)))
        self.base.camera.lookAt(Point3(*map(float, tgt)), Vec3(0, 0, 1))
        self.base.graphicsEngine.renderFrame()
        if not self.tex.hasRamImage():
            self.base.graphicsEngine.renderFrame()
        if not self.tex.hasRamImage():
            return None
        data = self.tex.getRamImageAs("RGBA").getData()
        w, h = self.tex.getXSize(), self.tex.getYSize()
        return QImage(data, w, h, 4 * w, QImage.Format_RGBA8888).mirrored(False, True)


def make_geom_node(name, v, n, c, idx, uv=None):
    """numpy arrays -> Panda3D NodePath (fast path through the buffer protocol, slow fallback)."""
    from panda3d.core import (Geom, GeomNode, GeomTriangles, GeomVertexArrayFormat, GeomVertexData,
                              GeomVertexFormat, GeomVertexWriter, InternalName, NodePath)
    arr = GeomVertexArrayFormat()
    arr.addColumn(InternalName.getVertex(), 3, Geom.NTFloat32, Geom.CPoint)
    arr.addColumn(InternalName.getNormal(), 3, Geom.NTFloat32, Geom.CNormal)
    arr.addColumn(InternalName.getColor(), 4, Geom.NTFloat32, Geom.CColor)
    if uv is not None:
        arr.addColumn(InternalName.getTexcoord(), 2, Geom.NTFloat32, Geom.CTexcoord)
    fmt = GeomVertexFormat()
    fmt.addArray(arr)
    fmt = GeomVertexFormat.registerFormat(fmt)
    vdata = GeomVertexData(name, fmt, Geom.UHStatic)
    cols = [v, n, c] + ([uv] if uv is not None else [])
    data = np.ascontiguousarray(np.concatenate(cols, axis=1).astype(np.float32))
    vdata.uncleanSetNumRows(len(v))
    try:
        handle = vdata.modifyArray(0)
        if handle.getArrayFormat().getStride() != data.shape[1] * 4:
            raise ValueError("unexpected stride")
        memoryview(handle).cast("B")[:] = data.tobytes()
    except Exception:
        vdata.setNumRows(len(v))
        wv = GeomVertexWriter(vdata, "vertex")
        wn = GeomVertexWriter(vdata, "normal")
        wc = GeomVertexWriter(vdata, "color")
        wt = GeomVertexWriter(vdata, "texcoord") if uv is not None else None
        for i in range(len(v)):
            wv.setData3f(*map(float, v[i]))
            wn.setData3f(*map(float, n[i]))
            wc.setData4f(*map(float, c[i]))
            if wt is not None:
                wt.setData2f(*map(float, uv[i]))
    prim = GeomTriangles(Geom.UHStatic)
    prim.setIndexType(Geom.NTUint32)
    idx = np.ascontiguousarray(idx.astype(np.uint32))
    try:
        iarr = prim.modifyVertices()
        iarr.uncleanSetNumRows(len(idx))
        memoryview(iarr).cast("B")[:] = idx.tobytes()
    except Exception:
        prim = GeomTriangles(Geom.UHStatic)
        prim.setIndexType(Geom.NTUint32)
        for i in range(0, len(idx), 3):
            prim.addVertices(int(idx[i]), int(idx[i + 1]), int(idx[i + 2]))
    geom = Geom(vdata)
    geom.addPrimitive(prim)
    node = GeomNode(name)
    node.addGeom(geom)
    return NodePath(node)


def _lines(name: str, polylines: list, color=(1, 1, 1, 1), thickness: float = 2.0):
    from panda3d.core import LineSegs
    ls = LineSegs(name)
    ls.setColor(*color)
    ls.setThickness(thickness)
    for pts in polylines:
        if len(pts) < 2:
            continue
        ls.moveTo(*map(float, pts[0]))
        for p in pts[1:]:
            ls.drawTo(*map(float, p))
    from panda3d.core import NodePath
    return NodePath(ls.create())


# ----------------------------------------------------------------------------------------------
class Viewport3D(QWidget):
    cursorMoved = Signal(float, float, float)

    def __init__(self, doc, settings, parent=None):
        super().__init__(parent)
        self.doc, self.settings = doc, settings
        self.setFocusPolicy(Qt.StrongFocus)
        self.setMouseTracking(True)
        self.setAcceptDrops(True)
        self.setMinimumSize(200, 150)
        self.world: PandaWorld | None = None
        self.error = ""
        self.image: QImage | None = None
        self.cam = G.OrbitCamera(np.array([0.0, 0.0, 0.0]))
        self.nodes: dict[int, object] = {}      # id(MapObject) -> NodePath
        self.rings: dict[int, object] = {}
        self.routes_np = None
        self.navmesh_np = None
        self.props_np = None
        self.show_navmesh = False
        self.show_props = True
        self.show_labels = True
        self.dirty = True
        self._keys: set[int] = set()
        self._mouse = None       # (button, last QPointF)
        self._drag = None
        self._timer = QTimer(self)
        self._timer.timeout.connect(self._tick)
        self._timer.start(16)
        if not HAVE_PANDA:
            self._fallback("Panda3D is not installed.\n\npip install panda3d\n\nThe 2D views work without it.")
            return
        try:
            self.world = PandaWorld.get()
        except Exception as e:  # pragma: no cover - depends on the GPU/driver
            self._fallback(f"Panda3D could not open an offscreen buffer:\n{e}")
            return
        doc.mapChanged.connect(lambda _id: self.rebuild())
        doc.profileReset.connect(self.rebuild)
        doc.contentChanged.connect(self._on_content)
        doc.selectionChanged.connect(self._on_selection)

    def _fallback(self, text: str) -> None:
        self.error = text
        lay = QVBoxLayout(self)
        lab = QLabel(text)
        lab.setAlignment(Qt.AlignCenter)
        lab.setStyleSheet("color: #ddd; background: #303030;")
        lay.addWidget(lab)

    # -- scene building -------------------------------------------------------------------------
    def _heightmap(self):
        sc = self.doc.scene()
        return sc.heightmap if sc else None

    def rebuild(self) -> None:
        if self.world is None:
            return
        w = self.world
        for root in (w.world_root, w.objects_root, w.overlay_root):
            root.getChildren().detach()
        self.nodes.clear()
        self.rings.clear()
        self.routes_np = self.navmesh_np = self.props_np = None
        m, sc = self.doc.map, self.doc.scene()
        if m is None or sc is None:
            self.dirty = True
            return
        hm = sc.heightmap
        if sc.gwmb is not None and sc.gwmb.terrain is not None:
            self._build_gwmb_terrain(sc.gwmb)
        elif hm is not None:
            terrain = make_geom_node("terrain", *G.terrain_mesh(hm))
            terrain.reparentTo(w.world_root)
        elif sc.path_graph is not None:
            flat = G.flat_mesh_from_trapezoids(sc.path_graph)
            if flat:
                make_geom_node("walkable", *flat).reparentTo(w.world_root)
        self._build_props(sc)
        self._build_navmesh(sc, hm)
        for o in m.objects:
            self._add_object(o)
        self._build_routes()
        self._on_selection(self.doc.selection)
        self.frame_all()

    def _build_gwmb_terrain(self, gm) -> None:
        from panda3d.core import Filename
        t = gm.terrain
        n = t.normals if t.normals is not None else np.tile([0, 0, 1.0], (len(t.vertices), 1))
        c = np.ones((len(t.vertices), 4), np.float32)
        uv = None if t.uvs is None else np.stack([t.uvs[:, 0], 1.0 - t.uvs[:, 1]], axis=1)
        node = make_geom_node("terrain", t.vertices, n.astype(np.float32), c, t.indices,
                              uv if t.texture is not None else None)
        if t.texture is not None:
            node.setTexture(self.world.base.loader.loadTexture(Filename.fromOsSpecific(str(t.texture))))
        node.setTwoSided(True)
        node.reparentTo(self.world.world_root)

    def _build_props(self, sc) -> None:
        from panda3d.core import Filename, Mat4
        root = self.world.world_root.attachNewNode("props")
        self.props_np = root
        if sc.gwmb is not None:
            protos = {}
            loader = self.world.base.loader
            for pl in sc.gwmb.placements:
                if pl.model_hash not in protos:
                    meshes = sc.gwmb.model(pl.model_hash)
                    proto = None
                    if meshes:
                        proto = root.attachNewNode(f"model_{pl.model_hash:x}")
                        proto.detachNode()
                        for i, mesh in enumerate(meshes):
                            nrm = mesh.normals if mesh.normals is not None else _vertex_normals(mesh)
                            col = np.ones((len(mesh.vertices), 4), np.float32)
                            uv = None if mesh.uvs is None else np.stack([mesh.uvs[:, 0], 1 - mesh.uvs[:, 1]], 1)
                            sub = make_geom_node(f"sub{i}", mesh.vertices, nrm, col, mesh.indices,
                                                 uv if mesh.texture is not None else None)
                            if mesh.texture is not None:
                                sub.setTexture(loader.loadTexture(Filename.fromOsSpecific(str(mesh.texture))))
                            sub.reparentTo(proto)
                    protos[pl.model_hash] = proto
                proto = protos[pl.model_hash]
                if proto is None:
                    continue
                inst = root.attachNewNode("prop")
                b, p = pl.basis, pl.position
                inst.setMat(Mat4(*b[:, 0], 0, *b[:, 1], 0, *b[:, 2], 0, *p, 1))
                proto.instanceTo(inst)
            root.setTwoSided(True)
        elif sc.ffna is not None and sc.ffna.props:
            centers = np.array([G.gw_to_render(p.x, p.y, p.z) for p in sc.ffna.props], np.float32)
            sizes = np.array([max(abs(p.scale), 0.2) * 120 for p in sc.ffna.props], np.float32)
            soup = G.box_soup(centers, sizes)
            if soup:
                make_geom_node("prop_boxes", *soup).reparentTo(root)
        root.setScale(1)
        root.show() if self.show_props else root.hide()

    def _build_navmesh(self, sc, hm) -> None:
        if sc.path_graph is None:
            return
        polys = []
        for plane in sc.path_graph.planes:
            for t in plane.trapezoids:
                pts = []
                for x, y in t.polygon() + [t.polygon()[0]]:
                    z = hm.height_at(x, y) if hm is not None else 0.0
                    pts.append((x, y, -z + 6))
                polys.append(pts)
        self.navmesh_np = _lines("navmesh", polys, (0.45, 1.0, 0.55, 1), 1.0)
        self.navmesh_np.reparentTo(self.world.overlay_root)
        self.navmesh_np.setDepthOffset(2)
        self.navmesh_np.show() if self.show_navmesh else self.navmesh_np.hide()

    def _add_object(self, o: MapObject) -> None:
        from panda3d.core import TextNode
        np_ = self.world.objects_root.attachNewNode(f"obj_{o.id}")
        self.world.pawn.instanceTo(np_)
        if isinstance(o, (TravelPortal, Area)):
            np_.setScale(1.6, 1.6, 0.6)
        elif isinstance(o, Waypoint):
            np_.setScale(0.6)
        label = TextNode("label")
        label.setText(o.display_name())
        label.setAlign(TextNode.ACenter)
        label.setTextColor(1, 1, 1, 1)
        label.setShadow(0.05, 0.05)
        label.setShadowColor(0, 0, 0, 1)
        lnp = np_.attachNewNode(label)
        lnp.setScale(55)
        lnp.setZ(260)
        lnp.setBillboardPointEye()
        lnp.setLightOff()
        lnp.setName("label")
        lnp.show() if self.show_labels else lnp.hide()
        self.nodes[id(o)] = np_
        self._sync_object(o)

    def _sync_object(self, o: MapObject) -> None:
        np_ = self.nodes.get(id(o))
        if np_ is None:
            return
        np_.setPos(*map(float, G.gw_to_render(o.x, o.y, o.z)))
        np_.setH(o.yaw)
        c = QColor(o.COLOR)
        sel = any(o is s for s in self.doc.selection)
        k = 1.35 if sel else 1.0
        np_.setColorScale(min(c.redF() * k, 1), min(c.greenF() * k, 1), min(c.blueF() * k, 1), 1)
        lab = np_.find("label")
        if not lab.isEmpty():
            lab.node().setText(o.display_name())
            lab.node().setTextColor(1, 1, 0.55, 1) if sel else lab.node().setTextColor(1, 1, 1, 1)
        old = self.rings.pop(id(o), None)
        if old is not None:
            old.removeNode()
        r = 0.0
        if isinstance(o, (Area, TravelPortal)):
            r = o.radius
        elif isinstance(o, AgentPlacement) and o.movement == "roam":
            r = o.roam_radius
        rings = []
        if r > 0:
            rings.append(G.ring_points(o.x, o.y, r, self._heightmap(), o.z))
        if sel:
            rings.append(G.ring_points(o.x, o.y, 90, self._heightmap(), o.z, 24))
        if rings:
            ring = _lines("ring", rings, (c.redF(), c.greenF(), c.blueF(), 1), 2.0 if not sel else 3.0)
            ring.reparentTo(self.world.overlay_root)
            self.rings[id(o)] = ring
        self.dirty = True

    def _build_routes(self) -> None:
        if self.routes_np is not None:
            self.routes_np.removeNode()
            self.routes_np = None
        m = self.doc.map
        if m is None:
            return
        lines = []
        for r in m.routes:
            wps = m.waypoints(r.id)
            pts = [G.gw_to_render(w.x, w.y, w.z) + np.array([0, 0, 30]) for w in wps]
            if r.loop and len(pts) > 2:
                pts.append(pts[0])
            lines.append(pts)
        if lines:
            self.routes_np = _lines("routes", lines, (1.0, 0.72, 0.3, 1), 3.0)
            self.routes_np.reparentTo(self.world.overlay_root)
        self.dirty = True

    # -- doc events -------------------------------------------------------------------------------
    def _on_content(self, kind, obj) -> None:
        if self.world is None or self.doc.map is None:
            return
        m = self.doc.map
        if isinstance(obj, MapObject):
            if kind == "added" and obj in m.objects and id(obj) not in self.nodes:
                self._add_object(obj)
            elif kind == "removed":
                n = self.nodes.pop(id(obj), None)
                if n is not None:
                    n.removeNode()
                r = self.rings.pop(id(obj), None)
                if r is not None:
                    r.removeNode()
            elif kind == "changed":
                self._sync_object(obj)
            if isinstance(obj, Waypoint):
                self._build_routes()
        elif obj is m:
            self.rebuild()
        else:
            self._build_routes()
        self.dirty = True

    def _on_selection(self, sel) -> None:
        if self.world is None or self.doc.map is None:
            return
        for o in self.doc.map.objects:
            if id(o) in self.nodes:
                self._sync_object(o)
        self.dirty = True

    def set_layer(self, name: str, on: bool) -> None:
        setattr(self, f"show_{name}", on)
        if name == "navmesh" and self.navmesh_np is not None:
            self.navmesh_np.show() if on else self.navmesh_np.hide()
        if name == "props" and self.props_np is not None:
            self.props_np.show() if on else self.props_np.hide()
        if name == "labels":
            for n in self.nodes.values():
                lab = n.find("label")
                if not lab.isEmpty():
                    lab.show() if on else lab.hide()
        self.dirty = True

    # -- camera ---------------------------------------------------------------------------------
    def frame_all(self) -> None:
        sc = self.doc.scene()
        m = self.doc.map
        if sc is None or m is None:
            return
        spawns = m.spawns()
        if spawns:  # start where a character would stand, like entering the map
            s = spawns[0]
            self.focus_point(s.x, s.y, s.z if s.z else self.doc.ground_z(s.x, s.y, 0.0), 4500)
        else:
            x0, y0, x1, y1 = sc.bounds()
            cx, cy = (x0 + x1) / 2, (y0 + y1) / 2
            self.focus_point(cx, cy, self.doc.ground_z(cx, cy, 0.0), max(x1 - x0, y1 - y0) * 0.8)

    def focus_point(self, x, y, z, distance=None) -> None:
        self.cam.target = G.gw_to_render(x, y, z)
        if distance:
            self.cam.distance = distance
        self.dirty = True

    def focus_objects(self, objs) -> None:
        objs = [o for o in objs if isinstance(o, MapObject)]
        if not objs:
            return
        x = sum(o.x for o in objs) / len(objs)
        y = sum(o.y for o in objs) / len(objs)
        z = sum(o.z for o in objs) / len(objs)
        spread = max([math.hypot(o.x - x, o.y - y) for o in objs] + [0])
        self.focus_point(x, y, z, max(1200.0, spread * 3))

    # -- rendering ------------------------------------------------------------------------------
    def resizeEvent(self, ev):
        self.cam.aspect = max(self.width(), 1) / max(self.height(), 1)
        self.dirty = True
        super().resizeEvent(ev)

    def _tick(self) -> None:
        if self.world is None or not self.isVisible():
            return
        if self._keys:
            speed = self.cam.distance * 0.02 + 20
            f = (Qt.Key_W in self._keys) - (Qt.Key_S in self._keys)
            r = (Qt.Key_D in self._keys) - (Qt.Key_A in self._keys)
            u = (Qt.Key_E in self._keys) - (Qt.Key_Q in self._keys)
            if f or r or u:
                self.cam.fly(f * speed, r * speed, u * speed)
                self.dirty = True
        if self.dirty:
            self.dirty = False
            try:
                self.image = self.world.render_image(self.cam)
            except Exception as e:  # pragma: no cover
                self.error = f"render failed: {e}"
            self.update()

    def paintEvent(self, ev):
        if self.world is None:
            return super().paintEvent(ev)
        p = QPainter(self)
        if self.image is not None:
            p.drawImage(self.rect(), self.image)
        else:
            p.fillRect(self.rect(), QColor("#303030"))
        p.setPen(QColor("white"))
        txt = "3D"
        if self.doc.armed is not None:
            txt += f"  |  placing: {self.doc.armed.name}  (right-click / Esc to stop)"
        if self.error:
            txt += "  |  " + self.error
        p.drawText(QPointF(8, 16), txt)
        p.end()

    # -- picking --------------------------------------------------------------------------------
    def _ndc(self, pos: QPointF) -> tuple[float, float]:
        return (2 * pos.x() / max(self.width(), 1) - 1, 1 - 2 * pos.y() / max(self.height(), 1))

    def ground_at(self, pos: QPointF):
        """GW (x, y, z) under the mouse, or None."""
        o, d = self.cam.ray(*self._ndc(pos))
        hit = G.ray_ground(o, d, self._heightmap(), plane_z_render=float(self.cam.target[2]))
        return None if hit is None else G.render_to_gw(hit)

    def object_at(self, pos: QPointF, radius_px: float = 16.0):
        m = self.doc.map
        if m is None:
            return None
        best, best_d = None, radius_px
        for o in m.objects:
            for lift in (40.0, 140.0):
                pr = self.cam.project(G.gw_to_render(o.x, o.y, o.z) + np.array([0, 0, lift]))
                if pr is None:
                    continue
                sx = (pr[0] + 1) / 2 * self.width()
                sy = (1 - pr[1]) / 2 * self.height()
                d = math.hypot(sx - pos.x(), sy - pos.y())
                if d < best_d:
                    best, best_d = o, d
        return best

    # -- input ----------------------------------------------------------------------------------
    def mousePressEvent(self, ev):
        self.setFocus()
        pos = ev.position()
        self._mouse = (ev.button(), pos)
        if ev.button() == Qt.RightButton and self.doc.armed is not None:
            self.doc.arm(None)
            self.dirty = True
            self._mouse = None
            return
        if ev.button() != Qt.LeftButton or self.world is None:
            return
        if self.doc.armed is not None:
            g = self.ground_at(pos)
            if g is not None:
                x, y = self._snap(g[0]), self._snap(g[1])
                self.doc.place_template(self.doc.armed, x, y, self.doc.ground_z(x, y, g[2]))
            return
        hit = self.object_at(pos)
        if hit is None:
            if not (ev.modifiers() & (Qt.ControlModifier | Qt.ShiftModifier)):
                self.doc.select([])
            return
        if ev.modifiers() & (Qt.ControlModifier | Qt.ShiftModifier):
            self.doc.select([hit], add=True)
            return
        if not any(hit is s for s in self.doc.selection):
            self.doc.select([hit])
        g = self.ground_at(pos)
        self._drag = {"anchor": hit, "grab": g, "orig": [(o, (o.x, o.y, o.z)) for o in self.doc.selected_objects()],
                      "key": self.doc.new_drag_key(), "moved": False}

    def mouseMoveEvent(self, ev):
        pos = ev.position()
        g = self.ground_at(pos) if self.world is not None else None
        if g is not None:
            self.cursorMoved.emit(*g)
        if self._mouse is None:
            return
        button, last = self._mouse
        d = pos - last
        self._mouse = (button, pos)
        if button == Qt.RightButton:
            self.cam.orbit(-d.x() * 0.3, -d.y() * 0.3)
            self.dirty = True
        elif button == Qt.MiddleButton:
            self.cam.pan(d.x() / max(self.width(), 1), d.y() / max(self.height(), 1))
            self.dirty = True
        elif button == Qt.LeftButton and self._drag is not None and g is not None and self._drag["grab"]:
            from ..content import commands as C
            dr = self._drag
            ax0, ay0, _ = next(v for o, v in dr["orig"] if o is dr["anchor"])
            nx = self._snap(ax0 + g[0] - dr["grab"][0])
            ny = self._snap(ay0 + g[1] - dr["grab"][1])
            dx, dy = nx - ax0, ny - ay0
            changes = []
            for o, (ox, oy, oz) in dr["orig"]:
                x, y = ox + dx, oy + dy
                changes.append((o, {"x": x, "y": y, "z": self.doc.ground_z(x, y, oz)}))
            self.doc.push(C.SetFields(changes, f"Move {len(changes)} object(s)", merge_key=dr["key"]))

    def mouseReleaseEvent(self, ev):
        self._mouse = None
        self._drag = None

    def mouseDoubleClickEvent(self, ev):
        hit = self.object_at(ev.position())
        if hit is not None:
            self.focus_objects([hit])

    def wheelEvent(self, ev):
        self.cam.dolly(0.85 ** (ev.angleDelta().y() / 120.0))
        self.dirty = True

    def keyPressEvent(self, ev):
        k = ev.key()
        if k in (Qt.Key_W, Qt.Key_A, Qt.Key_S, Qt.Key_D, Qt.Key_Q, Qt.Key_E) and not ev.modifiers():
            self._keys.add(k)
            return
        if k == Qt.Key_F:
            if self.doc.selection:
                self.focus_objects(self.doc.selection)
            else:
                self.frame_all()
            return
        if k == Qt.Key_Escape:
            self.doc.arm(None)
            self.doc.select([])
            self.dirty = True
            return
        if k in (Qt.Key_Z, Qt.Key_X) and not ev.modifiers():
            step = self.settings.angle_snap or 15.0
            self.doc.rotate_selected(step if k == Qt.Key_X else -step)
            return
        super().keyPressEvent(ev)

    def keyReleaseEvent(self, ev):
        if not ev.isAutoRepeat():
            self._keys.discard(ev.key())
        super().keyReleaseEvent(ev)

    def focusOutEvent(self, ev):
        self._keys.clear()
        super().focusOutEvent(ev)

    def _snap(self, v: float) -> float:
        if not self.settings.snap:
            return v
        g = max(float(self.settings.grid_size), 1.0)
        return round(v / g) * g

    # -- palette drops ----------------------------------------------------------------------------
    def dragEnterEvent(self, ev):
        if ev.mimeData().hasFormat(MIME_TEMPLATE) or ev.mimeData().hasFormat(MIME_AGENT):
            ev.acceptProposedAction()

    def dragMoveEvent(self, ev):
        self.dragEnterEvent(ev)

    def dropEvent(self, ev):
        g = self.ground_at(ev.position())
        if g is None:
            return
        x, y = self._snap(g[0]), self._snap(g[1])
        z = self.doc.ground_z(x, y, g[2])
        md = ev.mimeData()
        if md.hasFormat(MIME_TEMPLATE):
            from .. import library
            path = bytes(md.data(MIME_TEMPLATE)).decode()
            t = next((t for t in library.templates() if t.path == path), None)
            if t is not None:
                self.doc.place_template(t, x, y, z)
        elif md.hasFormat(MIME_AGENT):
            a = self.doc.profile.find("agent", bytes(md.data(MIME_AGENT)).decode())
            if a is not None:
                self.doc.place_agent_def(a, x, y, z)
        ev.acceptProposedAction()


def _vertex_normals(mesh) -> np.ndarray:
    v, i = mesh.vertices.astype(np.float32), mesh.indices.reshape(-1, 3)
    n = np.zeros_like(v)
    fn = np.cross(v[i[:, 1]] - v[i[:, 0]], v[i[:, 2]] - v[i[:, 0]])
    for k in range(3):
        np.add.at(n, i[:, k], fn)
    ln = np.linalg.norm(n, axis=1, keepdims=True)
    ln[ln == 0] = 1
    return (n / ln).astype(np.float32)
