"""Push a profile's spawn points into a running ASURA: write the SQL, apply it with psql, restart worlds.

Qt-free so it can be tested; ``asura_cs.ui.deploy_window`` is the GUI over it.

What ASURA does with it (checked against the ASURA source):

* ``asura-world`` reads ``map_config.spawn_x/spawn_y/spawn_plane`` at start-up (``main.rs``) and on
  travel; the extra ``map_spawn`` rows are stored but nothing reads them yet.
* ``asura-supervisor restart <service>`` asks the *running* supervisor (control port, default
  127.0.0.1:7711) to stop and start one service. It does not re-seed.
* ``asura-supervisor start`` **does** re-seed: it applies ``tools/map-config.sql``, whose upsert puts
  the stock spawn back for the catalogue maps it lists (Ascalon City 148 among them). After a full
  start, apply again.
"""

from __future__ import annotations

import os
import re
import shutil
import subprocess
import tempfile
from dataclasses import dataclass
from pathlib import Path

from .content.model import Profile
from .export import export_sql

_NO_WINDOW = 0x08000000 if os.name == "nt" else 0  # CREATE_NO_WINDOW: no console flash on Windows


def run(cmd: list[str], cwd: str | Path | None = None, timeout: int = 600) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, timeout=timeout,
                          creationflags=_NO_WINDOW)


# ----------------------------------------------------------------------------------------------- discovery
def read_env(asura_root: str | Path) -> dict[str, str]:
    """``KEY=value`` lines of ``<asura>/.env`` (quotes stripped, comments ignored)."""
    env: dict[str, str] = {}
    path = Path(asura_root) / ".env"
    if not path.exists():
        return env
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        line = line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        k, v = line.split("=", 1)
        env[k.strip()] = v.strip().strip("'").strip('"')
    return env


def database_url(asura_root: str | Path) -> str | None:
    return read_env(asura_root).get("ASURA_DATABASE_URL") or os.environ.get("ASURA_DATABASE_URL") or None


def mask_url(url: str) -> str:
    return re.sub(r"(://[^:/@]+:)([^@]*)@", r"\1****@", url or "")


def find_psql() -> str | None:
    """psql on PATH, else the newest ``C:\\Program Files\\PostgreSQL\\<ver>\\bin\\psql.exe`` (as ASURA's
    ``tools/seed-map.ps1`` does)."""
    p = shutil.which("psql")
    if p:
        return p
    root = Path(os.environ.get("ProgramFiles", r"C:\Program Files")) / "PostgreSQL"
    if root.is_dir():
        def ver(d: Path):
            return [int(x) if x.isdigit() else 0 for x in re.split(r"[.\-]", d.name)]
        for d in sorted((d for d in root.iterdir() if d.is_dir()), key=ver, reverse=True):
            exe = d / "bin" / "psql.exe"
            if exe.exists():
                return str(exe)
    return None


def supervisor_command(asura_root: str | Path) -> list[str]:
    """The built supervisor if there is one (fast), else ``cargo run -p asura-supervisor --``."""
    root = Path(asura_root)
    for build in ("debug", "release"):
        exe = root / "target" / build / ("asura-supervisor.exe" if os.name == "nt" else "asura-supervisor")
        if exe.exists():
            return [str(exe)]
    return ["cargo", "run", "-q", "-p", "asura-supervisor", "--"]


@dataclass
class Service:
    name: str
    exe: str
    is_world: bool


def services(asura_root: str | Path) -> list[Service]:
    """Services from ``config/services.toml`` (worlds are the ones running asura-world)."""
    path = Path(asura_root) / "config" / "services.toml"
    if not path.exists():
        return []
    try:
        import tomllib
    except ModuleNotFoundError:  # Python < 3.11
        return _services_fallback(path.read_text(encoding="utf-8"))
    data = tomllib.loads(path.read_text(encoding="utf-8"))
    out = []
    for name, svc in (data.get("services") or {}).items():
        exe = str(svc.get("exe", ""))
        out.append(Service(name, exe, "asura-world" in exe or name.startswith("world")))
    return out


def _services_fallback(text: str) -> list[Service]:
    out = []
    for m in re.finditer(r'^\[services\.([^\]]+)\]\s*\nexe\s*=\s*"([^"]*)"', text, re.M):
        out.append(Service(m.group(1), m.group(2), "asura-world" in m.group(2) or m.group(1).startswith("world")))
    return out


# ----------------------------------------------------------------------------------------------- data
@dataclass
class SpawnRow:
    map_id: int
    name: str
    new: tuple[float, float, int] | None      # profile spawn 0
    current: tuple[float, float, int] | None  # database map_config spawn
    count: int                                # spawns in the profile
    portals_changed: bool = False             # the profile's doors differ from map_portal
    portal_count: int = 0

    @property
    def spawn_changed(self) -> bool:
        if self.new is None or self.current is None:
            return False
        return (abs(self.new[0] - self.current[0]) > 0.05 or abs(self.new[1] - self.current[1]) > 0.05
                or self.new[2] != self.current[2])

    @property
    def changed(self) -> bool:
        if self.current is None:
            return False  # not in the database's catalogue: nothing to apply to
        return self.spawn_changed or self.portals_changed


def current_spawns(psql: str, url: str) -> dict[int, tuple[float, float, int]]:
    r = run([psql, "-X", "-At", "-F", "\t", "-v", "ON_ERROR_STOP=1",
             "-c", "SELECT map_id, spawn_x, spawn_y, spawn_plane FROM map_config", url], timeout=60)
    if r.returncode != 0:
        raise RuntimeError(r.stderr.strip() or "psql failed")
    out = {}
    for line in r.stdout.splitlines():
        parts = line.split("\t")
        if len(parts) == 4 and parts[1] and parts[2]:
            out[int(parts[0])] = (float(parts[1]), float(parts[2]), int(parts[3] or 0))
    return out


PortalKey = tuple


def _door_key(x, y, to, ax, ay) -> tuple:
    r = lambda v: None if v is None else round(float(v), 1)  # noqa: E731
    to = int(to) if to else None
    if not to or (not ax and not ay):
        ax = ay = None
    return (r(x), r(y), to, r(ax), r(ay))


def profile_doors(m) -> list[tuple]:
    from .content.model import TravelPortal
    return sorted(_door_key(o.x, o.y, o.to_map, o.arrive_x, o.arrive_y)
                  for o in m.objects if isinstance(o, TravelPortal))


def current_portals(psql: str, url: str) -> dict[int, list[tuple]] | None:
    """map_portal as it stands, or None when the table does not exist yet (schema not applied)."""
    r = run([psql, "-X", "-At", "-F", "\t", "-v", "ON_ERROR_STOP=1",
             "-c", "SELECT map_id, x, y, to_map, arrive_x, arrive_y FROM map_portal", url], timeout=60)
    if r.returncode != 0:
        return None
    out: dict[int, list[tuple]] = {}
    for line in r.stdout.splitlines():
        parts = line.split("\t")
        if len(parts) != 6:
            continue
        num = lambda v: float(v) if v else None  # noqa: E731
        out.setdefault(int(parts[0]), []).append(
            _door_key(num(parts[1]), num(parts[2]), num(parts[3]), num(parts[4]), num(parts[5])))
    return {k: sorted(v) for k, v in out.items()}


def spawn_rows(profile: Profile, current: dict[int, tuple[float, float, int]],
               portals: dict[int, list[tuple]] | None = None) -> list[SpawnRow]:
    from .content.scope import in_scope
    rows = []
    for mid, m in sorted(profile.maps.items()):
        if not in_scope(mid):
            continue
        sp = m.spawns()
        new = (float(sp[0].x), float(sp[0].y), int(sp[0].plane)) if sp else None
        doors = profile_doors(m)
        portals_changed = portals is None or portals.get(mid, []) != doors
        rows.append(SpawnRow(mid, m.name, new, current.get(mid), len(sp), portals_changed, len(doors)))
    return rows


def sql_for_maps(profile: Profile, map_ids: list[int]) -> str:
    """The exporter's SQL restricted to ``map_ids``."""
    with tempfile.TemporaryDirectory() as d:
        out = Path(d) / "spawns.sql"
        export_sql(profile, out, map_ids=list(map_ids))
        return out.read_text(encoding="utf-8")


def apply_sql(psql: str, url: str, sql: str) -> subprocess.CompletedProcess:
    """``psql -v ON_ERROR_STOP=1 -q -f <file> <url>``: the same call ASURA's supervisor makes."""
    with tempfile.NamedTemporaryFile("w", suffix=".sql", delete=False, encoding="utf-8") as f:
        f.write(sql)
        name = f.name
    try:
        return run([psql, "-X", "-v", "ON_ERROR_STOP=1", "-q", "-f", name, url], timeout=120)
    finally:
        Path(name).unlink(missing_ok=True)


def restart(asura_root: str | Path, service: str) -> subprocess.CompletedProcess:
    return run(supervisor_command(asura_root) + ["restart", service], cwd=asura_root, timeout=900)


def status(asura_root: str | Path) -> subprocess.CompletedProcess:
    return run(supervisor_command(asura_root) + ["status"], cwd=asura_root, timeout=900)
