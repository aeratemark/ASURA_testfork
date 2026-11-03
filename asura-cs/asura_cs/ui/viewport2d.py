"""The 2D map views: Nox-style top-down and orthographic front/side.

GW world -> scene coordinates per view::

    TOP    scene = ( x, -y)    north up
    FRONT  scene = ( x,  z)    GW z grows downwards, like screen y
    SIDE   scene = ( y,  z)

Object glyphs ignore the view transform (constant pixel size, readable at any zoom); radii (roam,
trigger areas, portal reach) are drawn in world units.
"""

from __future__ import annotations

import math

import numpy as np
from PySide6.QtCore import QPointF, QRect, QRectF, QSize, Qt, Signal
from PySide6.QtGui import (QBrush, QColor, QFont, QImage, QPainter, QPainterPath, QPen, QPixmap, QPolygonF,
                           QTransform)
from PySide6.QtWidgets import (QGraphicsEllipseItem, QGraphicsItem, QGraphicsPathItem, QGraphicsPixmapItem,
                               QGraphicsScene, QGraphicsView, QMenu, QRubberBand)

from ..content.model import AgentPlacement, Area, MapObject, TravelPortal, Waypoint

TOP, FRONT, SIDE = "top", "front", "side"
MIME_TEMPLATE = "application/x-asura-cs-template"
MIME_AGENT = "application/x-asura-cs-agent"

BG = QColor("#26282b")
GRID_MINOR = QColor("#303338")
GRID_MAJOR = QColor("#3d4148")
AXIS = QColor("#5a6270")


def to_scene(mode: str, x: float, y: float, z: float) -> QPointF:
    if mode == TOP:
        return QPointF(x, -y)
    if mode == FRONT:
        return QPointF(x, z)
    return QPointF(y, z)


def from_scene(mode: str, p: QPointF, base: tuple[float, float, float]) -> tuple[float, float, float]:
    """Scene point -> GW (x, y, z); the axis the view does not show is taken from ``base``."""
    x, y, z = base
    if mode == TOP:
        return p.x(), -p.y(), z
    if mode == FRONT:
        return p.x(), y, p.y()
    return x, p.x(), p.y()


# ----------------------------------------------------------------------------------------------
class ObjectItem(QGraphicsItem):
    R = 7.0

    def __init__(self, view: "MapView2D", obj: MapObject):
        super().__init__()
        self.view, self.obj = view, obj
        self.selected = False
        self.setFlag(QGraphicsItem.ItemIgnoresTransformations, True)
        self.setZValue(10)
        self.setAcceptHoverEvents(True)
        self.setCursor(Qt.PointingHandCursor)
        self.sync()

    def sync(self):
        self.prepareGeometryChange()
        o = self.obj
        self.setPos(to_scene(self.view.mode, o.x, o.y, o.z))
        self.setToolTip(f"{o.TYPE_LABEL}: {o.display_name()}\n{o.id}\n({o.x:.0f}, {o.y:.0f}, {o.z:.0f})")
        self.update()

    def boundingRect(self) -> QRectF:
        return QRectF(-60, -24, 160, 48)

    def shape(self) -> QPainterPath:
        p = QPainterPath()
        p.addEllipse(QPointF(0, 0), self.R + 3, self.R + 3)
        return p

    def paint(self, painter: QPainter, option, widget=None):
        o = self.obj
        painter.setRenderHint(QPainter.Antialiasing)
        color = QColor(o.COLOR)
        if self.selected:
            painter.setPen(QPen(QColor("#ffffff"), 2))
            painter.setBrush(Qt.NoBrush)
            painter.drawEllipse(QPointF(0, 0), self.R + 4, self.R + 4)
        # facing
        if self.view.mode == TOP:
            fx, fy = o.facing_vector()
            painter.setPen(QPen(color.lighter(130), 2))
            painter.drawLine(QPointF(0, 0), QPointF(fx * 18, -fy * 18))
        painter.setPen(QPen(QColor("#101010"), 1))
        painter.setBrush(QBrush(color))
        if isinstance(o, TravelPortal):
            painter.drawRect(QRectF(-self.R, -self.R, 2 * self.R, 2 * self.R))
        elif isinstance(o, Waypoint):
            painter.drawPolygon(QPolygonF([QPointF(0, -self.R), QPointF(self.R, 0), QPointF(0, self.R),
                                           QPointF(-self.R, 0)]))
        else:
            painter.drawEllipse(QPointF(0, 0), self.R, self.R)
        painter.setPen(QColor("#101010"))
        f = QFont(painter.font())
        f.setPointSizeF(6.5)
        f.setBold(True)
        painter.setFont(f)
        painter.drawText(QRectF(-self.R, -self.R, 2 * self.R, 2 * self.R), Qt.AlignCenter, o.ICON)
        if self.view.show_labels:
            f.setBold(False)
            f.setPointSizeF(7.5)
            painter.setFont(f)
            painter.setPen(QColor("#e8e8e8") if not self.selected else QColor("#ffffa0"))
            painter.drawText(QPointF(self.R + 4, 4), o.display_name())


# ----------------------------------------------------------------------------------------------
class MapView2D(QGraphicsView):
    cursorMoved = Signal(float, float, float)
    focusRequested = Signal(object)          # a MapObject to frame in other views

    def __init__(self, doc, settings, mode: str = TOP, parent=None):
        super().__init__(parent)
        self.doc, self.settings, self.mode = doc, settings, mode
        self.setScene(QGraphicsScene(self))
        self.scene().setSceneRect(QRectF(-200000, -200000, 400000, 400000))
        self.setRenderHints(QPainter.Antialiasing | QPainter.SmoothPixmapTransform)
        self.setViewportUpdateMode(QGraphicsView.SmartViewportUpdate)
        self.setTransformationAnchor(QGraphicsView.AnchorUnderMouse)
        self.setResizeAnchor(QGraphicsView.AnchorViewCenter)
        self.setHorizontalScrollBarPolicy(Qt.ScrollBarAlwaysOff)
        self.setVerticalScrollBarPolicy(Qt.ScrollBarAlwaysOff)
        self.setMouseTracking(True)
        self.setAcceptDrops(True)
        self.setFocusPolicy(Qt.StrongFocus)
        self.setBackgroundBrush(BG)

        self.show_grid = True
        self.show_terrain = True
        self.show_navmesh = True
        self.show_navgraph = False
        self.show_props = True
        self.show_labels = True
        self.follow_terrain = True

        self.items_by_obj: dict[int, ObjectItem] = {}
        self.radius_items: dict[int, QGraphicsEllipseItem] = {}
        self.route_items: list[QGraphicsPathItem] = []
        self.layer_items: dict[str, QGraphicsItem] = {}

        self._drag = None        # dict while moving/rotating selection
        self._pan = None         # last pos while panning
        self._band = None        # (origin, QRubberBand)
        self._space = False

        doc.mapChanged.connect(lambda _id: self.rebuild())
        doc.profileReset.connect(self.rebuild)
        doc.contentChanged.connect(self._on_content)
        doc.selectionChanged.connect(self._on_selection)
        self.scale(0.05, 0.05)

    # -- building -----------------------------------------------------------------------------
    def set_mode(self, mode: str) -> None:
        if mode != self.mode:
            self.mode = mode
            self.rebuild()
            self.frame_all()

    def rebuild(self) -> None:
        sc = self.scene()
        sc.clear()
        self.items_by_obj.clear()
        self.radius_items.clear()
        self.route_items.clear()
        self.layer_items.clear()
        m = self.doc.map
        if m is None:
            return
        scene = self.doc.scene()
        if scene is not None and self.mode == TOP:
            self._build_terrain(scene)
            self._build_navmesh(scene)
            self._build_props(scene)
        for o in m.objects:
            self._add_item(o)
        self._rebuild_routes()
        self._apply_layer_visibility()
        self._on_selection(self.doc.selection)

    def _build_terrain(self, scene) -> None:
        hm = scene.heightmap
        if hm is None:
            return
        img = hillshade(hm.z)
        item = QGraphicsPixmapItem(QPixmap.fromImage(img))
        item.setTransformationMode(Qt.SmoothTransformation)
        item.setPos(hm.bounds.min_x, -hm.bounds.max_y)
        item.setTransform(QTransform.fromScale(hm.dx, hm.dy))
        item.setZValue(-30)
        self.scene().addItem(item)
        self.layer_items["terrain"] = item

    def _build_navmesh(self, scene) -> None:
        g = scene.path_graph
        if g is None:
            return
        mesh = QPainterPath()
        mesh.setFillRule(Qt.WindingFill)
        graph = QPainterPath()
        for plane in g.planes:
            traps = plane.trapezoids
            for i, t in enumerate(traps):
                mesh.addPolygon(QPolygonF([QPointF(px, -py) for px, py in t.polygon()]))
                mesh.closeSubpath()
                cx, cy = t.center()
                for n in t.neighbours():
                    if i < n < len(traps):  # each adjacency once
                        nx, ny = traps[n].center()
                        graph.moveTo(cx, -cy)
                        graph.lineTo(nx, -ny)
        has_terrain = "terrain" in self.layer_items
        mi = QGraphicsPathItem(mesh)
        mi.setBrush(QColor(90, 160, 110, 70 if has_terrain else 150))
        pen = QPen(QColor(120, 200, 140, 120))
        pen.setCosmetic(True)
        mi.setPen(pen)
        mi.setZValue(-20)
        self.scene().addItem(mi)
        self.layer_items["navmesh"] = mi
        gi = QGraphicsPathItem(graph)
        pen = QPen(QColor(255, 210, 90, 140))
        pen.setCosmetic(True)
        gi.setPen(pen)
        gi.setZValue(-19)
        self.scene().addItem(gi)
        self.layer_items["navgraph"] = gi

    def _build_props(self, scene) -> None:
        path = QPainterPath()
        if scene.gwmb is not None:
            pts = [(p.position[0], p.position[1]) for p in scene.gwmb.placements]
        elif scene.ffna is not None:
            pts = [(p.x, p.y) for p in scene.ffna.props]
        else:
            pts = []
        for x, y in pts:
            path.addRect(QRectF(x - 40, -y - 40, 80, 80))
        if pts:
            it = QGraphicsPathItem(path)
            it.setBrush(QColor(170, 170, 180, 120))
            pen = QPen(QColor(200, 200, 210, 160))
            pen.setCosmetic(True)
            it.setPen(pen)
            it.setZValue(-10)
            it.setToolTip("Scenery props from the map blob (read-only)")
            self.scene().addItem(it)
            self.layer_items["props"] = it

    def _apply_layer_visibility(self) -> None:
        vis = {"terrain": self.show_terrain, "navmesh": self.show_navmesh, "navgraph": self.show_navgraph,
               "props": self.show_props}
        for k, it in self.layer_items.items():
            it.setVisible(vis.get(k, True))
        self.viewport().update()

    def set_layer(self, name: str, on: bool) -> None:
        setattr(self, f"show_{name}", on)
        if name in ("labels", "grid"):
            for it in self.items_by_obj.values():
                it.update()
            self.viewport().update()
        else:
            self._apply_layer_visibility()

    def _add_item(self, o: MapObject) -> None:
        it = ObjectItem(self, o)
        self.scene().addItem(it)
        self.items_by_obj[id(o)] = it
        self._sync_radius(o)

    def _sync_radius(self, o: MapObject) -> None:
        r = 0.0
        color = QColor(o.COLOR)
        if isinstance(o, Area):
            r = o.radius
        elif isinstance(o, TravelPortal):
            r = o.radius
        elif isinstance(o, AgentPlacement) and o.movement == "roam":
            r = o.roam_radius
        old = self.radius_items.pop(id(o), None)
        if old is not None:
            self.scene().removeItem(old)
        if r > 0 and self.mode == TOP:
            c = to_scene(TOP, o.x, o.y, o.z)
            e = QGraphicsEllipseItem(QRectF(c.x() - r, c.y() - r, 2 * r, 2 * r))
            pen = QPen(color, 1.5, Qt.DashLine)
            pen.setCosmetic(True)
            e.setPen(pen)
            fill = QColor(color)
            fill.setAlpha(28)
            e.setBrush(fill)
            e.setZValue(-5)
            self.scene().addItem(e)
            self.radius_items[id(o)] = e

    def _rebuild_routes(self) -> None:
        for it in self.route_items:
            self.scene().removeItem(it)
        self.route_items = []
        m = self.doc.map
        if m is None:
            return
        for r in m.routes:
            wps = m.waypoints(r.id)
            if len(wps) < 2:
                continue
            path = QPainterPath(to_scene(self.mode, wps[0].x, wps[0].y, wps[0].z))
            for w in wps[1:]:
                path.lineTo(to_scene(self.mode, w.x, w.y, w.z))
            if r.loop:
                path.lineTo(to_scene(self.mode, wps[0].x, wps[0].y, wps[0].z))
            it = QGraphicsPathItem(path)
            pen = QPen(QColor(Waypoint.COLOR), 2, Qt.DashDotLine)
            pen.setCosmetic(True)
            it.setPen(pen)
            it.setZValue(5)
            it.setToolTip(f"Route {r.id}")
            self.scene().addItem(it)
            self.route_items.append(it)

    # -- document events ------------------------------------------------------------------------
    def _on_content(self, kind: str, obj) -> None:
        m = self.doc.map
        if m is None:
            return
        if isinstance(obj, MapObject):
            if kind == "added" and obj in m.objects and id(obj) not in self.items_by_obj:
                self._add_item(obj)
            elif kind == "removed":
                it = self.items_by_obj.pop(id(obj), None)
                if it is not None:
                    self.scene().removeItem(it)
                r = self.radius_items.pop(id(obj), None)
                if r is not None:
                    self.scene().removeItem(r)
            elif kind == "changed" and id(obj) in self.items_by_obj:
                self.items_by_obj[id(obj)].sync()
                self._sync_radius(obj)
            if isinstance(obj, Waypoint):
                self._rebuild_routes()
        elif obj is m or (kind in ("added", "removed") and hasattr(obj, "map_id")):
            self.rebuild()
        else:
            self._rebuild_routes()

    def _on_selection(self, sel: list) -> None:
        ids = {id(s) for s in sel}
        for key, it in self.items_by_obj.items():
            on = key in ids
            if it.selected != on:
                it.selected = on
                it.setZValue(11 if on else 10)
                it.update()

    # -- framing ------------------------------------------------------------------------------------
    def frame_all(self) -> None:
        m = self.doc.map
        if m is None:
            return
        sc = self.doc.scene()
        if self.mode == TOP and sc is not None:
            x0, y0, x1, y1 = sc.bounds()
            rect = QRectF(QPointF(x0, -y1), QPointF(x1, -y0))
        else:
            pts = [to_scene(self.mode, o.x, o.y, o.z) for o in m.objects] or [QPointF(0, 0)]
            xs, ys = [p.x() for p in pts], [p.y() for p in pts]
            rect = QRectF(QPointF(min(xs) - 2000, min(ys) - 2000), QPointF(max(xs) + 2000, max(ys) + 2000))
        self.fitInView(rect, Qt.KeepAspectRatio)

    def frame_objects(self, objs: list) -> None:
        objs = [o for o in objs if isinstance(o, MapObject)]
        if not objs:
            return
        pts = [to_scene(self.mode, o.x, o.y, o.z) for o in objs]
        xs, ys = [p.x() for p in pts], [p.y() for p in pts]
        pad = 1500
        self.fitInView(QRectF(QPointF(min(xs) - pad, min(ys) - pad), QPointF(max(xs) + pad, max(ys) + pad)),
                       Qt.KeepAspectRatio)

    # -- drawing --------------------------------------------------------------------------------
    def drawBackground(self, painter: QPainter, rect: QRectF) -> None:
        painter.fillRect(rect, BG)
        if not self.show_grid:
            return
        scale = self.transform().m11()
        step = max(float(self.settings.grid_size), 1.0)
        while step * scale < 8:
            step *= 10
        major = step * 10
        for spacing, color in ((step, GRID_MINOR), (major, GRID_MAJOR)):
            pen = QPen(color)
            pen.setCosmetic(True)
            painter.setPen(pen)
            left = math.floor(rect.left() / spacing) * spacing
            top = math.floor(rect.top() / spacing) * spacing
            x = left
            lines = []
            while x < rect.right():
                lines.append((QPointF(x, rect.top()), QPointF(x, rect.bottom())))
                x += spacing
            y = top
            while y < rect.bottom():
                lines.append((QPointF(rect.left(), y), QPointF(rect.right(), y)))
                y += spacing
            for a, b in lines:
                painter.drawLine(a, b)
        pen = QPen(AXIS, 1)
        pen.setCosmetic(True)
        painter.setPen(pen)
        painter.drawLine(QPointF(rect.left(), 0), QPointF(rect.right(), 0))
        painter.drawLine(QPointF(0, rect.top()), QPointF(0, rect.bottom()))

    def drawForeground(self, painter: QPainter, rect: QRectF) -> None:
        painter.save()
        painter.resetTransform()
        painter.setPen(QColor("#c8c8c8"))
        label = {TOP: "TOP  (x east, y north)", FRONT: "FRONT  (x, z)", SIDE: "SIDE  (y, z)"}[self.mode]
        if self.doc.armed is not None:
            label += f"   |   placing: {self.doc.armed.name}  (right-click / Esc to stop)"
        painter.drawText(QPointF(8, 16), label)
        painter.restore()

    # -- helpers --------------------------------------------------------------------------------
    def _snap(self, v: float) -> float:
        if not self.settings.snap:
            return v
        g = max(float(self.settings.grid_size), 1.0)
        return round(v / g) * g

    def _world(self, pos, base=(0.0, 0.0, 0.0)) -> tuple[float, float, float]:
        return from_scene(self.mode, self.mapToScene(pos), base)

    def _item_at(self, pos) -> ObjectItem | None:
        for it in self.items(pos):
            if isinstance(it, ObjectItem):
                return it
        return None

    # -- mouse ----------------------------------------------------------------------------------
    def mousePressEvent(self, ev):
        self.setFocus()
        if ev.button() == Qt.MiddleButton or (ev.button() == Qt.LeftButton and self._space):
            self._pan = ev.position()
            self.setCursor(Qt.ClosedHandCursor)
            return
        if ev.button() == Qt.RightButton:
            if self.doc.armed is not None:
                self.doc.arm(None)
                self.viewport().update()
                return
            self._context_menu(ev)
            return
        if ev.button() != Qt.LeftButton:
            return
        mods = ev.modifiers()
        if self.doc.armed is not None:
            if self.mode != TOP:
                self.doc.message.emit("Place objects in the Top view or the 3D view.")
                return
            x, y, _ = self._world(ev.position().toPoint())
            self.doc.place_template(self.doc.armed, self._snap(x), self._snap(y))
            return
        hit = self._item_at(ev.position().toPoint())
        if hit is not None:
            if mods & (Qt.ControlModifier | Qt.ShiftModifier):
                self.doc.select([hit.obj], add=True)
                return
            if not any(hit.obj is s for s in self.doc.selection):
                self.doc.select([hit.obj])
            objs = self.doc.selected_objects()
            self._drag = {
                "rotate": bool(mods & Qt.AltModifier),
                "anchor": hit.obj,
                "start": self.mapToScene(ev.position().toPoint()),
                "orig": [(o, (o.x, o.y, o.z, o.yaw)) for o in objs],
                "key": self.doc.new_drag_key(),
            }
            return
        origin = ev.position().toPoint()
        band = QRubberBand(QRubberBand.Rectangle, self.viewport())
        band.setGeometry(QRect(origin, QSize()))
        band.show()
        self._band = (origin, band, bool(mods & (Qt.ShiftModifier | Qt.ControlModifier)))

    def mouseMoveEvent(self, ev):
        p = ev.position().toPoint()
        base = (0.0, 0.0, 0.0)
        x, y, z = self._world(p, base)
        if self.mode == TOP:
            z = self.doc.ground_z(x, y, 0.0)
        self.cursorMoved.emit(x, y, z)
        if self._pan is not None:
            d = ev.position() - self._pan
            self._pan = ev.position()
            self.horizontalScrollBar().setValue(self.horizontalScrollBar().value() - int(d.x()))
            self.verticalScrollBar().setValue(self.verticalScrollBar().value() - int(d.y()))
            return
        if self._band is not None:
            origin, band, _ = self._band
            band.setGeometry(QRect(origin, p).normalized())
            return
        if self._drag is not None:
            self._drag_to(self.mapToScene(p))

    def _drag_to(self, scene_pos: QPointF) -> None:
        d = self._drag
        from ..content import commands as C
        if d["rotate"]:
            if self.mode != TOP:
                return
            changes = []
            for o, (ox, oy, oz, oyaw) in d["orig"]:
                c = to_scene(TOP, ox, oy, oz)
                ang = math.degrees(math.atan2(-(scene_pos.y() - c.y()), scene_pos.x() - c.x()))
                if self.settings.snap and self.settings.angle_snap > 0:
                    ang = round(ang / self.settings.angle_snap) * self.settings.angle_snap
                changes.append((o, {"yaw": ang % 360.0}))
            self.doc.push(C.SetFields(changes, "Rotate", merge_key=d["key"]))
            return
        delta = scene_pos - d["start"]
        anchor_orig = next(v for o, v in d["orig"] if o is d["anchor"])
        a_scene = to_scene(self.mode, *anchor_orig[:3]) + delta
        ax, ay, az = from_scene(self.mode, a_scene, anchor_orig[:3])
        if self.mode == TOP:
            ax, ay = self._snap(ax), self._snap(ay)
        elif self.mode == FRONT:
            ax, az = self._snap(ax), self._snap(az)
        else:
            ay, az = self._snap(ay), self._snap(az)
        dx, dy, dz = ax - anchor_orig[0], ay - anchor_orig[1], az - anchor_orig[2]
        changes = []
        for o, (ox, oy, oz, _) in d["orig"]:
            nx, ny, nz = ox + dx, oy + dy, oz + dz
            if self.mode == TOP and self.follow_terrain:
                nz = self.doc.ground_z(nx, ny, oz)
            changes.append((o, {"x": nx, "y": ny, "z": nz}))
        self.doc.push(C.SetFields(changes, f"Move {len(changes)} object(s)", merge_key=d["key"]))

    def mouseReleaseEvent(self, ev):
        if self._pan is not None:
            self._pan = None
            self.unsetCursor()
            return
        if self._band is not None:
            origin, band, add = self._band
            rect = band.geometry()
            band.hide()
            band.deleteLater()
            self._band = None
            if rect.width() < 3 and rect.height() < 3:
                if not add:
                    self.doc.select([])
                return
            area = self.mapToScene(rect).boundingRect()
            hits = [it.obj for it in self.items_by_obj.values() if area.contains(it.pos())]
            if add:
                self.doc.select(self.doc.selection + [h for h in hits if all(h is not s for s in self.doc.selection)])
            else:
                self.doc.select(hits)
            return
        self._drag = None

    def mouseDoubleClickEvent(self, ev):
        hit = self._item_at(ev.position().toPoint())
        if hit is not None:
            self.focusRequested.emit(hit.obj)

    def wheelEvent(self, ev):
        steps = ev.angleDelta().y() / 120.0
        f = 1.2 ** steps
        s = self.transform().m11() * f
        if 1e-4 < s < 50:
            self.scale(f, f)

    def keyPressEvent(self, ev):
        if ev.key() == Qt.Key_Space and not ev.isAutoRepeat():
            self._space = True
            self.setCursor(Qt.OpenHandCursor)
            return
        if ev.key() == Qt.Key_Escape:
            self.doc.arm(None)
            self.doc.select([])
            self.viewport().update()
            return
        if ev.key() == Qt.Key_F:
            if self.doc.selection:
                self.frame_objects(self.doc.selection)
            else:
                self.frame_all()
            return
        super().keyPressEvent(ev)

    def keyReleaseEvent(self, ev):
        if ev.key() == Qt.Key_Space and not ev.isAutoRepeat():
            self._space = False
            self.unsetCursor()
            return
        super().keyReleaseEvent(ev)

    def _context_menu(self, ev):
        hit = self._item_at(ev.position().toPoint())
        if hit is not None and not any(hit.obj is s for s in self.doc.selection):
            self.doc.select([hit.obj])
        menu = QMenu(self)
        has = bool(self.doc.selected_objects())
        x, y, _ = self._world(ev.position().toPoint())
        menu.addAction("Duplicate", self.doc.duplicate_selected).setEnabled(has)
        menu.addAction("Delete", self.doc.delete_selected).setEnabled(has)
        menu.addAction("Drop to ground", self.doc.drop_to_ground).setEnabled(has)
        menu.addAction("Rotate +45°", lambda: self.doc.rotate_selected(45)).setEnabled(has)
        menu.addSeparator()
        if has:
            menu.addAction("Show in 3D", lambda: self.focusRequested.emit(self.doc.selected_objects()[0]))
        menu.addAction(f"Copy coordinates ({x:.0f}, {y:.0f})", lambda: _copy(f"{x:.1f}, {y:.1f}"))
        menu.exec(ev.globalPosition().toPoint())

    # -- drag & drop from the palette -------------------------------------------------------------
    def dragEnterEvent(self, ev):
        if ev.mimeData().hasFormat(MIME_TEMPLATE) or ev.mimeData().hasFormat(MIME_AGENT):
            ev.acceptProposedAction()

    def dragMoveEvent(self, ev):
        self.dragEnterEvent(ev)

    def dropEvent(self, ev):
        if self.mode != TOP:
            self.doc.message.emit("Drop objects into the Top view or the 3D view.")
            return
        x, y, _ = self._world(ev.position().toPoint())
        x, y = self._snap(x), self._snap(y)
        md = ev.mimeData()
        if md.hasFormat(MIME_TEMPLATE):
            from .. import library
            path = bytes(md.data(MIME_TEMPLATE)).decode()
            t = next((t for t in library.templates() if t.path == path), None)
            if t is not None:
                self.doc.place_template(t, x, y)
        elif md.hasFormat(MIME_AGENT):
            aid = bytes(md.data(MIME_AGENT)).decode()
            a = self.doc.profile.find("agent", aid)
            if a is not None:
                self.doc.place_agent_def(a, x, y)
        ev.acceptProposedAction()


def _copy(text: str) -> None:
    from PySide6.QtWidgets import QApplication
    QApplication.clipboard().setText(text)


def hillshade(z: np.ndarray) -> QImage:
    """Height-tinted hillshade of a GW heightmap (z negative = up) as an RGB QImage."""
    up = -z.astype(np.float32)
    gy, gx = np.gradient(up)
    slope = np.pi / 2 - np.arctan(np.hypot(gx, gy) / 8.0)
    aspect = np.arctan2(-gx, gy)
    az, alt = np.radians(315), np.radians(45)
    shade = np.sin(alt) * np.sin(slope) + np.cos(alt) * np.cos(slope) * np.cos(az - aspect)
    shade = np.clip(shade, 0, 1)
    lo, hi = np.percentile(up, 2), np.percentile(up, 98)
    t = np.clip((up - lo) / max(hi - lo, 1e-3), 0, 1)
    low = np.array([70, 92, 60], np.float32)
    high = np.array([170, 160, 130], np.float32)
    rgb = (low + (high - low) * t[..., None]) * (0.45 + 0.75 * shade[..., None])
    rgb = np.ascontiguousarray(np.clip(rgb, 0, 255).astype(np.uint8))
    h, w, _ = rgb.shape
    buf = rgb.tobytes()
    img = QImage(buf, w, h, 3 * w, QImage.Format_RGB888)
    return img.copy()  # detach from the temporary buffer
