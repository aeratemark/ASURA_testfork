"""The Qt-free half of 'Apply to ASURA', with a stand-in psql."""
import os
import stat
import sys
from pathlib import Path

import pytest

from asura_cs import deploy
from asura_cs.content.model import MapContent, Profile, SpawnPoint


def _profile():
    p = Profile(id="t", name="T")
    for mid, x in ((148, 9600.0), (146, 6602.0), (145, 1.0)):
        m = MapContent(mid, f"map {mid}")
        m.objects.append(SpawnPoint(id=f"s{mid}", x=x, y=100.0, index=0))
        p.maps[mid] = m
    return p


def test_env_and_mask(tmp_path):
    (tmp_path / ".env").write_text('# c\nASURA_DATABASE_URL="postgres://asura:s3cret@127.0.0.1:5432/asura"\n')
    url = deploy.database_url(tmp_path)
    assert url == "postgres://asura:s3cret@127.0.0.1:5432/asura"
    assert deploy.mask_url(url) == "postgres://asura:****@127.0.0.1:5432/asura"


def test_rows_mark_changed_and_missing():
    current = {148: (9541.873, 4796.456, 0), 146: (6602.0, 100.0, 0)}
    rows = {r.map_id: r for r in deploy.spawn_rows(_profile(), current, portals={})}
    assert rows[148].changed and not rows[146].changed
    assert rows[145].current is None and not rows[145].changed
    # no map_portal table yet: every map's portals count as changed
    rows = {r.map_id: r for r in deploy.spawn_rows(_profile(), current, portals=None)}
    assert rows[146].portals_changed and rows[146].changed


def test_sql_is_restricted_to_selected_maps():
    sql = deploy.sql_for_maps(_profile(), [148])
    assert "map_id = 148" in sql and "146" not in sql and sql.strip().endswith("COMMIT;")


@pytest.fixture
def fake_psql(tmp_path):
    """A 'psql' that answers the SELECT and records -f files."""
    log = tmp_path / "calls.txt"
    script = tmp_path / "psql_fake.py"
    script.write_text(
        "import sys\n"
        f"open({str(log)!r}, 'a').write(' '.join(sys.argv[1:]) + '\\n')\n"
        "if '-c' in sys.argv: print('148\\t9541.873\\t4796.456\\t0'); print('146\\t6602\\t4485\\t0')\n"
        "if '-f' in sys.argv: open(" + repr(str(tmp_path / 'applied.sql')) + ", 'w').write(open(sys.argv[sys.argv.index('-f') + 1]).read())\n")
    if os.name == "nt":
        exe = tmp_path / "psql.bat"
        exe.write_text(f'@"{sys.executable}" "{script}" %*\n')
    else:
        exe = tmp_path / "psql"
        exe.write_text(f'#!/bin/sh\nexec "{sys.executable}" "{script}" "$@"\n')
        exe.chmod(exe.stat().st_mode | stat.S_IEXEC)
    return str(exe), tmp_path


def test_current_spawns_and_apply_with_fake_psql(fake_psql):
    exe, tmp = fake_psql
    cur = deploy.current_spawns(exe, "postgres://x")
    assert cur[148] == (9541.873, 4796.456, 0) and cur[146][1] == 4485.0
    r = deploy.apply_sql(exe, "postgres://x", deploy.sql_for_maps(_profile(), [148]))
    assert r.returncode == 0
    assert "INSERT INTO map_spawn" in (tmp / "applied.sql").read_text()
    assert "ON_ERROR_STOP=1" in (tmp / "calls.txt").read_text()


ASURA = Path(os.environ.get("ASURA_ROOT", Path(__file__).resolve().parents[2] / "src" / "ASURA"))


@pytest.mark.skipif(not (ASURA / "config").is_dir(), reason="no ASURA checkout")
def test_services_from_asura_config():
    names = {s.name: s.is_world for s in deploy.services(ASURA)}
    assert names.get("world-8001") is True and names.get("auth") is False
    assert deploy.supervisor_command(ASURA)[0] in ("cargo",) or "asura-supervisor" in deploy.supervisor_command(ASURA)[0]
