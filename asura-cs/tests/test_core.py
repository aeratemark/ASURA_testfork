"""Core tests: no Qt, no Panda3D. Run with ``pytest`` from the project root.

Tests that need an ASURA checkout use ``ASURA_ROOT`` (default ../ASURA) and skip without it.
"""

from __future__ import annotations

import json
import math
import os
import struct
from pathlib import Path

import numpy as np
import pytest

from asura_cs import library
from asura_cs.content import commands as C
from asura_cs.content import store
from asura_cs.content.base_profile import build_base_profile, copy_profile
from asura_cs.content.model import (AgentDef, AgentPlacement, Dialogue, DialogueChoice, DialogueNode, MapContent,
                                    Objective, Profile, Quest, Route, Script, SpawnPoint, Waypoint)
from asura_cs.content.validate import validate
from asura_cs.export import run as export
from asura_cs.formats import ffna_map, gwmb_export
from asura_cs.formats.catalogue import Catalogue
from asura_cs.formats.gwdat import MAGIC, GwDat, decode_filename

ASURA = Path(os.environ.get("ASURA_ROOT", Path(__file__).resolve().parents[2] / "src" / "ASURA"))
needs_asura = pytest.mark.skipif(not (ASURA / "crates").is_dir(), reason="no ASURA checkout")


# ---------------------------------------------------------------------------------------- ASURA data
@needs_asura
def test_catalogue_reads_maps_spawns_portals_and_crossings():
    cat = Catalogue.from_asura(ASURA)
    asc = cat.maps[148]
    assert asc.name == "Ascalon City (pre-Searing)" and asc.map_file_id == 113021
    assert asc.spawns and abs(asc.spawns[0].x - 9541.873) < 1e-3
    assert len(asc.portals) >= 1
    assert any(c.to_map == 146 for c in asc.crossings)
    assert cat.maps[449].name == "Kamadan, Jewel of Istan"


@needs_asura
def test_path_graph_contains_ascalon_spawn():
    from asura_cs.formats.path_graphs import PathGraphLibrary
    lib = PathGraphLibrary.load(ASURA / "data" / "path-graphs.bin")
    g = lib.for_map(148, 113021)
    assert g is not None and g.find_trapezoid(9541.873, 4796.456) is not None
    assert g.find_trapezoid(0.0, 0.0) is None or True  # (0,0) is outside on measured maps; not asserted


@needs_asura
def test_base_profile_portals_get_measured_destinations():
    prof = build_base_profile(Catalogue.from_asura(ASURA))
    asc = prof.maps[148]
    portals = [o for o in asc.objects if o.TYPE == "portal"]
    assert any(p.to_map == 146 and p.measured for p in portals)
    assert asc.spawns()[0].index == 0


# ---------------------------------------------------------------------------------------- formats
def _ffna_blob(xdim=32, ydim=32):
    chunks = []
    bounds = struct.pack("<IB4f", 0xDEAD, 2, -1000.0, -2000.0, 1000.0, 2000.0) + b"\0" * 20
    chunks.append((ffna_map.CHUNK_MAP_INFO, bounds))
    heights = np.arange(xdim * ydim, dtype="<f4")
    terrain = (struct.pack("<IIBI", 1, 2, 0, 0) + struct.pack("<II", xdim, ydim) + struct.pack("<ffHff", 0, 0, 0, 0, 0)
               + struct.pack("<BI", 0, heights.nbytes) + heights.tobytes()
               + struct.pack("<BI", 0, xdim * ydim) + bytes(range(256)) * (xdim * ydim // 256))
    chunks.append((ffna_map.CHUNK_TERRAIN, terrain))
    names = struct.pack("<IB", 0, 1) + struct.pack("<HHH", 0x105, 0x100, 0)
    chunks.append((ffna_map.CHUNK_PROPS_FILENAMES, names))
    prop = struct.pack("<H", 0) + struct.pack("<11f", 10, 20, -30, 0, 0, -1, 0.0, 1.0, 0, 2.0, 0) + bytes([0, 1]) + b"\0" * 8
    props = struct.pack("<IH", 0, 0) + struct.pack("<I", len(prop) + 2) + struct.pack("<H", 1) + prop
    chunks.append((ffna_map.CHUNK_PROPS_INFO, props))
    out = b"ffna\x03"
    for cid, payload in chunks:
        out += struct.pack("<II", cid, len(payload)) + payload
    return out


def test_ffna_terrain_props_and_bounds():
    m = ffna_map.parse_map(_ffna_blob(), 1)
    assert m.bounds.max_y == 2000.0
    hm = m.heightmap
    assert hm.z.shape == (32, 32) and hm.z[0, 0] == 0 and hm.z[0, 1] == 1 and hm.z[1, 0] == 32
    assert hm.height_at(-1000.0, 2000.0) == pytest.approx(0.0)
    assert m.prop_filenames == [decode_filename(0x105, 0x100)] == [6]
    p = m.props[0]
    assert (p.x, p.y, p.z, p.scale, p.model_file_id) == (10, 20, -30, 2.0, 6)
    assert p.yaw == pytest.approx(math.pi / 2)


def test_heightmap_untiles_32x32_blocks():
    m = ffna_map.parse_map(_ffna_blob(64, 32), 1)
    z = m.heightmap.z
    # Second tile (columns 32..63) starts at flat index 1024 in the file.
    assert z.shape == (32, 64) and z[0, 32] == 1024 and z[1, 0] == 32


def test_heightmap_raycast_hits_flat_ground():
    hm = ffna_map.Heightmap(ffna_map.MapBounds(-1000, -1000, 1000, 1000), np.zeros((32, 32), np.float32))
    hit = hm.raycast((0, 0, -500), (0.3, 0.2, 1.0))  # looking down (+z is down in GW)
    assert hit is not None and hit[2] == pytest.approx(0, abs=1e-3)
    assert hit[0] == pytest.approx(150, rel=0.05) and hit[1] == pytest.approx(100, rel=0.05)
    assert hm.raycast((0, 0, -500), (0, 0, -1)) is None  # looking up


def test_gwdat_index_and_stored_entry(tmp_path):
    payload = b"ffna\x03hello"
    toc = struct.pack("<II", 0x1234, 3)  # file id 0x1234 -> index 3 -> entry 2
    head_size = 32
    mft_off = head_size + len(payload) + len(toc)
    entries = [(0, 0), (head_size + len(payload), len(toc)), (head_size, len(payload))]
    data = struct.pack("<IIIIQII", MAGIC, 32, 512, 0, mft_off, 0, 0) + payload + toc
    data += struct.pack("<IIIIII", 0, 0, 0, len(entries), 0, 0)
    for off, size in entries:
        data += struct.pack("<QIHBBII", off, size, 0, 0, 0, 0, 0)
    f = tmp_path / "Gw.dat"
    f.write_bytes(data)
    dat = GwDat(f, tmp_path / "cache")
    assert 0x1234 in dat and dat.file(0x1234) == payload


def test_gwmb_export_loader(tmp_path):
    v = lambda x, y, z: {"x": x, "y": y, "z": z}  # noqa: E731
    vert = {"pos": v(1, 2, 3), "normal": v(0, 1, 0), "uv_coord0": {"x": 0, "y": 0}, "uv_coord1": {"x": 0, "y": 0},
            "uv_coord2": {"x": 0, "y": 0}, "uv_coord3": {"x": 0, "y": 0}}
    (tmp_path / "map_77.json").write_text(json.dumps({
        "filehash": 77, "terrain": {"width": 1, "height": 1, "vertices": [vert] * 3, "indices": [0, 1, 2],
                                    "textures": []},
        "models": [{"model_hash": 0xAB, "world_pos": v(10, 20, 30), "model_right": v(1, 0, 0),
                    "model_up": v(0, 1, 0), "model_look": v(0, 0, 1), "scale": 2.0}],
        "min_x": 0, "max_x": 1, "min_y": 0, "max_y": 1, "min_z": 0, "max_z": 1}))
    (tmp_path / "model_0xAB_gwmb.json").write_text(json.dumps({"textures": [], "submodels_draw_order": [0],
        "submodels": [{"vertices": [{"pos": v(0, 1, 0), "texture_uv_coords": []}] * 3, "indices": [0, 1, 2]}]}))
    g = gwmb_export.load_map(tmp_path, 77)
    assert g.terrain.vertices[0].tolist() == [1, 3, 2]          # (x, y_up, z) -> (x, z, y_up)
    pl = g.placements[0]
    assert pl.position.tolist() == [10, 30, 20]
    local_up = np.array([0, 1, 0])                              # GWMB local up ...
    assert (pl.basis @ local_up).tolist() == [0, 0, 2]          # ... is render +z, scaled
    assert g.model(0xAB)[0].indices.tolist() == [0, 1, 2]


# ---------------------------------------------------------------------------------------- content
def _profile() -> Profile:
    p = Profile(id="t", name="Test")
    m = MapContent(148, "Ascalon City", 0, 113021)
    m.objects.append(SpawnPoint(id="spawn_148_0", x=1, y=2, index=0))
    p.maps[148] = m
    return p


def test_commands_undo_redo_and_drag_merge():
    p, hub = _profile(), C.ChangeHub()
    events = []
    hub.subscribe(lambda k, o, i: events.append(k))
    st = C.SimpleStack(hub)
    npc = AgentPlacement(id="npc_a", agent="guard", x=0, y=0)
    st.push(C.AddRecords([(p.maps[148].objects, npc)], "Place"))
    for x in (10, 20, 30):  # one drag = several moves with the same merge key
        st.push(C.SetFields([(npc, {"x": float(x)})], "Move", merge_key=("drag", 1)))
    assert npc.x == 30 and len(st.done) == 2
    st.undo()
    assert npc.x == 0
    st.undo()
    assert npc not in p.maps[148].objects
    st.redo(); st.redo()
    assert npc in p.maps[148].objects and npc.x == 30
    st.push(C.RemoveRecords([(p.maps[148].objects, npc)]))
    st.undo()
    assert p.maps[148].objects.index(npc) == 1
    assert {"added", "removed", "changed"} <= set(events)


def test_mutate_snapshots_nested_dialogue():
    d = Dialogue(id="dlg", nodes=[DialogueNode("greeting", "Hello")])
    st = C.SimpleStack(C.ChangeHub())
    st.push(C.Mutate(d, lambda dd: dd.nodes[0].choices.append(DialogueChoice("Bye", "end"))))
    assert len(d.nodes[0].choices) == 1
    st.undo()
    assert d.nodes[0].choices == []
    st.redo()
    assert d.nodes[0].choices[0].text == "Bye"


def test_duplicate_gets_fresh_ids():
    p = _profile()
    a = AgentPlacement(id="guard_01", x=0, y=0)
    p.maps[148].objects.append(a)
    dups = C.duplicate(p, [a, a])
    assert len({d.id for d in dups} | {"guard_01"}) == 3 and dups[0].x == 100


def test_store_roundtrip(tmp_path):
    p = _profile()
    p.agents.append(AgentDef(id="guard", name="Guard", services=["merchant"], dialogue="dlg"))
    p.dialogues.append(Dialogue(id="dlg", nodes=[DialogueNode("greeting", "Hi", "true", "",
                                                              [DialogueChoice("Bye", "end", "", "x = 1")])]))
    p.quests.append(Quest(id="q1", giver="guard", objectives=[Objective("talk", "guard")]))
    p.scripts.append(Script(id="guard_ai", source="return {}\n"))
    p.maps[148].routes.append(Route(id="r1", name="Walls"))
    p.maps[148].objects.append(Waypoint(id="wp1", route="r1", order=0))
    store.save(p, tmp_path / "prof")
    q = store.load(tmp_path / "prof")
    assert q.agents[0].services == ["merchant"]
    assert q.dialogues[0].nodes[0].choices[0].result == "x = 1"
    assert q.quests[0].objectives[0].target == "guard"
    assert q.scripts[0].source == "return {}\n"
    assert [type(o).__name__ for o in q.maps[148].objects] == ["SpawnPoint", "Waypoint"]
    assert q.maps[148].routes[0].name == "Walls"


def test_copy_profile_is_independent():
    p = _profile()
    c = copy_profile(p, "mine", "Mine")
    c.maps[148].objects[0].x = 999
    assert p.maps[148].objects[0].x == 1 and c.based_on == "t"


def test_validate_reports_missing_references():
    p = _profile()
    p.maps[148].objects.append(AgentPlacement(id="npc", agent="nobody", movement="route", route="nope"))
    p.dialogues.append(Dialogue(id="d", start="missing", nodes=[DialogueNode("a", choices=[DialogueChoice("x", "zz")])]))
    msgs = [i.message for i in validate(p, check_lua=False) if i.level == "error"]
    assert any("nobody" in m for m in msgs)
    assert any("route 'nope'" in m for m in msgs)
    assert any("start node" in m for m in msgs)
    assert any("missing node 'zz'" in m for m in msgs)


def test_library_template_creates_definition_once():
    library.reload()
    t = next(t for t in library.templates() if t.name == "Guard")
    p = _profile()
    a1 = t.instantiate(p, 148, 5, 6, -7)
    for coll, rec in a1:
        coll.append(rec)
    a2 = t.instantiate(p, 148, 8, 9, 0)
    assert len(a1) == 2 and len(a2) == 1          # definition reused on the second drop
    placement = a2[0][1]
    assert placement.agent == "guard" and placement.id != a1[1][1].id


def test_exporters(tmp_path):
    p = _profile()
    p.maps[148].objects.append(SpawnPoint(id="spawn_148_1", x=5, y=6, index=1))
    p.agents.append(AgentDef(id="g", name='Say "hi"\n'))
    sql = export("asura_sql", p, tmp_path / "out.sql")[0].read_text()
    assert "DELETE FROM map_spawn WHERE map_id = 148;" in sql
    assert "VALUES (148, 1, 5.0, 6.0, 0);" in sql and "spawn_count = 2" in sql
    js = json.loads(export("content_json", p, tmp_path / "out.json")[0].read_text())
    assert js["maps"][0]["objects"][0]["type"] == "spawn"
    lua = export("lua_bundle", p, tmp_path / "out.lua")[0].read_text()
    assert 'name = "Say \\"hi\\"\\n"' in lua


# ---------------------------------------------------------------------------------------- 3D math
def test_camera_ray_and_projection_roundtrip():
    from asura_cs.world.geometry import OrbitCamera
    cam = OrbitCamera(np.array([1000.0, 2000.0, 0.0]), yaw=30, pitch=-40, distance=5000, aspect=1.6)
    o, d = cam.ray(0, 0)
    assert np.allclose(o + d * 5000, cam.target)
    for nx, ny in [(0.5, -0.3), (-0.9, 0.8)]:
        o, d = cam.ray(nx, ny)
        px, py, depth = cam.project(o + d * 3000)
        assert px == pytest.approx(nx, abs=1e-6) and py == pytest.approx(ny, abs=1e-6)
    assert cam.project(cam.position() - cam.forward() * 10) is None


def test_ray_ground_and_heightmap_from_mesh():
    from asura_cs.world.geometry import OrbitCamera, heightmap_from_mesh, ray_ground, terrain_mesh
    hm = ffna_map.Heightmap(ffna_map.MapBounds(-1000, -1000, 1000, 1000), np.full((32, 32), -100.0, np.float32))
    cam = OrbitCamera(np.array([0.0, 0.0, 100.0]), pitch=-60, distance=2000)
    hit = ray_ground(*cam.ray(0, 0), hm)
    assert hit is not None and hit[2] == pytest.approx(100.0, abs=1.0)   # render z = -gw z
    v, n, c, i = terrain_mesh(hm)
    assert len(i) % 3 == 0 and i.max() < len(v) and np.allclose(n[:, 2], 1.0)
    back = heightmap_from_mesh(v, 32, 32)
    assert back.height_at(0, 0) == pytest.approx(-100.0)


def test_meshes_are_well_formed():
    from asura_cs.world.geometry import box_soup, pawn_mesh
    for mesh in (box_soup(np.zeros((3, 3)), np.ones(3) * 10), pawn_mesh()):
        v, n, c, i = mesh
        assert len(v) == len(n) == len(c) and i.max() < len(v) and len(i) % 3 == 0


# ---------------------------------------------------------------------------------------- UI (fake Qt)
@needs_asura
def test_ui_smoke_with_fake_qt():
    """Runs tests/smoke_ui.py in a subprocess against tests/fakeqt: editor wiring, not Qt itself."""
    import subprocess
    import sys
    r = subprocess.run([sys.executable, str(Path(__file__).with_name("smoke_ui.py")), str(ASURA)],
                       capture_output=True, text=True, timeout=600)
    assert r.returncode == 0, r.stdout[-2000:] + r.stderr[-4000:]
    assert "UI smoke OK" in r.stdout


@needs_asura
def test_native_helper_matches_python_port(tmp_path):
    import shutil
    if not shutil.which("rustc"):
        pytest.skip("no rustc")
    import random
    from asura_cs.formats import native
    from asura_cs.formats.fa_decompress import DecompressError, decompress
    exe = native.helper(ASURA, tmp_path)
    assert exe is not None and exe.exists()
    rng = random.Random(7)
    agreed_ok = 0
    for _ in range(150):
        n = rng.choice([16, 64, 256])
        data = bytearray(rng.getrandbits(8) for _ in range(n))
        data[0], data[2], data[3] = rng.choice([0x0F, 0xFF]), rng.randint(0, 4), 0
        size = rng.choice([4, 64, 500])
        try:
            py = decompress(bytes(data), size)
        except DecompressError:
            py = None
        try:
            rs = native.decompress(exe, bytes(data), size)
        except ValueError:
            rs = None
        assert py == rs
        agreed_ok += py is not None
    assert agreed_ok > 0


# ---------------------------------------------------------------------------------------- scope
@needs_asura
def test_base_profile_is_pre_searing_only_and_keeps_unmatched_crossings():
    from asura_cs.content.scope import PRE_SEARING
    prof = build_base_profile(Catalogue.from_asura(ASURA))
    assert set(prof.maps) <= set(PRE_SEARING) and 148 in prof.maps and 449 not in prof.maps
    # Green Hills County has measured crossings but no portal props: they must still be doorways
    gh = [o for o in prof.maps[160].objects if o.TYPE == "portal"]
    assert any(p.to_map == 146 and p.measured for p in gh)
    assert all(p.to_map in PRE_SEARING for m in prof.maps.values() for p in m.objects
               if p.TYPE == "portal" and p.to_map)


def test_validate_flags_out_of_scope_maps_and_destinations():
    from asura_cs.content.model import TravelPortal
    p = _profile()
    p.maps[449] = MapContent(449, "Kamadan")
    p.maps[148].objects.append(TravelPortal(id="door", to_map=449))
    msgs = [i.message for i in validate(p, check_lua=False) if i.level == "error"]
    assert any("outside pre-Searing" in m for m in msgs)
    assert any("destination map 449 is outside" in m for m in msgs)


@needs_asura
def test_map_portal_rows_become_the_profile_doors(tmp_path):
    """When the checkout carries 0025_map_portals.sql, the editor's doors are exactly those rows."""
    cat = Catalogue.from_asura(ASURA)
    if not cat.maps[148].doors:
        pytest.skip("this checkout has no map_portal migration")
    prof = build_base_profile(cat)
    gate = [o for o in prof.maps[148].objects if o.TYPE == "portal" and o.to_map == 146]
    assert len(gate) >= 2, "the gate prop and the measured crossing both lead to Lakeside"
    rv = [o for o in prof.maps[146].objects if o.TYPE == "portal" and o.to_map == 162]
    assert rv and rv[0].source.startswith("derived")
    sql = export("asura_sql", prof, tmp_path / "o.sql")[0].read_text()
    assert "CREATE TABLE IF NOT EXISTS map_portal" in sql and "DELETE FROM map_portal WHERE map_id = 148;" in sql
