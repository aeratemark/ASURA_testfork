"""Drive the editor's UI layer against tests/fakeqt (see that module's docstring)."""
import os
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE / "fakeqt"))
sys.path.insert(0, str(HERE.parent))
os.environ["HOME"] = tempfile.mkdtemp()  # settings go to a scratch home

from asura_cs.gamedata import Settings  # noqa: E402
from asura_cs.ui import main_window as mw  # noqa: E402
from asura_cs.content import store  # noqa: E402
from asura_cs.content.base_profile import build_base_profile, copy_profile  # noqa: E402
from asura_cs.content.model import AgentPlacement, Dialogue, Quest, Script  # noqa: E402
from asura_cs import library  # noqa: E402

asura = sys.argv[1]
s = Settings(asura_root=asura)
win = mw.MainWindow(s)
win.reload_gamedata(reset_view=False)
prof = copy_profile(build_base_profile(win.gamedata.catalogue), "test", "Test")
folder = Path(tempfile.mkdtemp()) / "test"
store.save(prof, folder)
win.open_profile(folder)
doc = win.doc
assert doc.map_id == 148, doc.map_id

# place from the palette (template), duplicate, move via SetFields drag merging, undo
guard = next(t for t in library.templates() if t.name == "Guard")
placed = doc.place_template(guard, 9600.0, 4800.0)
assert placed and isinstance(placed[0], AgentPlacement)
doc.duplicate_selected()
assert len(doc.selection) == 1 and doc.selection[0] is not placed[0]
win.v2d._drag = {"rotate": False, "anchor": placed[0], "start": mw.MapView2D.__module__ and __import__("PySide6.QtCore").QtCore.QPointF(0, 0),
                 "orig": [(placed[0], (placed[0].x, placed[0].y, placed[0].z, 0.0))], "key": doc.new_drag_key()}
QPointF = sys.modules["PySide6.QtCore"].QPointF
win.v2d._drag_to(QPointF(100, -50))
win.v2d._drag_to(QPointF(200, -100))
assert (placed[0].x, placed[0].y) == (9800.0, 4900.0), (placed[0].x, placed[0].y)
n = doc.undo_stack.count()
doc.undo_stack.undo()
assert (placed[0].x, placed[0].y) == (9600.0, 4800.0)
doc.undo_stack.redo()

# records: dialogue / quest / script editors
d = Dialogue(id="guard_talk", name="Guard talk")
from asura_cs.content.model import DialogueNode, DialogueChoice
d.nodes = [DialogueNode("greeting", "Halt! Who goes there?", choices=[DialogueChoice("A friend.", "end")])]
from asura_cs.content import commands as C
doc.push(C.AddRecords([(doc.profile.dialogues, d)], "add"))
win.open_record(d)
win.dialogues.nodes.currentRow = lambda: 0      # fake list widget has no selection model
win.dialogues.add_choice()
assert len(d.nodes[0].choices) == 2
win.dialogues._set_choice(1, "kind", "quest_available")
assert d.nodes[0].choices[1].kind == "quest_available"
q = Quest(id="q_first", name="First")
doc.push(C.AddRecords([(doc.profile.quests, q)], "add"))
win.open_record(q)
win.quests.add()
win.quests.table.currentRow = lambda: 0
win.quests._set(0, "target", "gate_guard")
assert q.objectives[0].target == "gate_guard"
doc.undo_stack.undo()
assert q.objectives[0].target == ""
doc.undo_stack.redo()
assert len(q.objectives) == 1
sc = Script(id="guard_ai", source="return {}")
doc.push(C.AddRecords([(doc.profile.scripts, sc)], "add"))
win.open_record(sc)

# properties: build for a placement and a definition; commit a field through the panel
doc.select([placed[0]])
win.props._flush() if win.props._pending else None
win.props._commit("name", "Gate Guard", [placed[0]])
assert placed[0].name == "Gate Guard"
adef = doc.profile.find("agent", placed[0].agent)
doc.select([adef])
win.props._flush() if win.props._pending else None
assert doc.rename(adef, "gate_guard") is None and placed[0].agent == "gate_guard"

# hierarchy + validation + export + save/reload
win.hierarchy.rebuild()
win.validate()
doc.delete_selected()
win.save()
again = store.load(folder)
assert any(o.name == "Gate Guard" for o in again.maps[148].objects)
assert again.dialogues[0].nodes[0].choices[1].kind == "quest_available"
for key in mw.EXPORTERS:
    out = folder.parent / f"out_{key}"
    mw.run_export(key, again, out)
# routes: arm the waypoint template and click three points
from asura_cs.content.model import Route
r = Route(id="wall_patrol", name="Wall patrol")
doc.push(C.AddRecords([(doc.map.routes, r)], "route"))
t = mw.WaypointTemplate("wall_patrol")
for i in range(3):
    doc.place_template(t, 9000.0 + i * 300, 4700.0)
wps = doc.map.waypoints("wall_patrol")
assert [w.order for w in wps] == [0, 1, 2] and len({w.id for w in wps}) == 3
from asura_cs.content.validate import validate
assert not [i for i in validate(doc.profile, win.gamedata.path_graphs) if "wall_patrol" in i.where and i.level == "error"]
win.add_map  # exists
win.v2d.set_mode("front"); win.v2d.set_mode("side"); win.v2d.set_mode("top")
# deploy window: no .env in the checkout here, so it only fills the table from the profile
from asura_cs.ui.deploy_window import DeployWindow
dw = DeployWindow(s, doc.profile, str(folder))
dw.rows = __import__("asura_cs.deploy", fromlist=["x"]).spawn_rows(doc.profile, {148: (0.0, 0.0, 0)})
dw.only_changed.isChecked = lambda: True
dw._fill_tree()
assert dw.svc_checks and dw.svc_checks[0].text  # world-8001 found
print("UI smoke OK:", len(doc.profile.maps), "maps;", doc.undo_stack.count(), "undo steps")
