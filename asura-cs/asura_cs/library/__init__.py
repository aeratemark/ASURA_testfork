"""Python content definitions: the Object Palette's templates.

A template is a recipe for something you can drop into a map. Templates are plain Python, so
anyone can add a file to a library folder::

    # my_library/ascalon.py
    from asura_cs.library import agent_template

    agent_template("Ascalon/Guards", "Ascalon Guard",
                   agent=dict(id="ascalon_guard", name="Ascalon Guard", category="npc",
                              allegiance="ally", level=15, profession="warrior", behaviour="guard"),
                   placement=dict(movement="stand"))

Library folders searched, in order: the built-in ``asura_cs/library/builtin.py``, ``~/.asura_cs/library``,
and ``<profile>/library``. Later definitions with the same path replace earlier ones.

Templates do not invent game facts: a template without a known GW model id leaves
``model_file_id = 0`` and the validator says so.
"""

from __future__ import annotations

import importlib.util
import sys
import traceback
from dataclasses import dataclass, field
from pathlib import Path
from typing import Callable

from ..content.model import OBJECT_TYPES, AgentDef, AgentPlacement, slug


@dataclass
class Template:
    path: str                 # "Category/Sub/Name" - the palette tree
    name: str
    description: str = ""
    object_type: str = "marker"   # a key of OBJECT_TYPES
    defaults: dict = field(default_factory=dict)
    agent: dict | None = None     # for agent templates: AgentDef fields (created on first use)
    model_file_id: int = 0        # for palette previews / filtering
    tags: tuple = ()

    def instantiate(self, profile, map_id: int, x: float, y: float, z: float = 0.0):
        """Return the (collection, record) pairs to add for one drop at (x, y, z)."""
        adds = []
        values = dict(self.defaults)
        if self.agent is not None:
            aid = self.agent.get("id") or slug(self.name)
            adef = profile.find("agent", aid)
            if adef is None:
                adef = AgentDef(**{**self.agent, "id": aid})
                adds.append((profile.agents, adef))
            values.setdefault("name", adef.name)
            values["agent"] = adef.id
            cls = AgentPlacement
        else:
            cls = OBJECT_TYPES[self.object_type]
        obj = cls(**{k: v for k, v in values.items() if k != "id"})
        obj.id = profile.unique_id(values.get("id") or f"{cls.TYPE}_{self.name}")
        obj.x, obj.y, obj.z = x, y, z
        adds.append((profile.maps[map_id].objects, obj))
        return adds


_REGISTRY: dict[str, Template] = {}
_ERRORS: list[str] = []


def register(t: Template) -> Template:
    _REGISTRY[t.path] = t
    return t


def object_template(path: str, name: str, object_type: str, description: str = "", **defaults) -> Template:
    return register(Template(f"{path}/{name}", name, description, object_type, defaults))


def agent_template(path: str, name: str, agent: dict, placement: dict | None = None,
                   description: str = "", tags: tuple = ()) -> Template:
    return register(Template(f"{path}/{name}", name, description, "placement", placement or {}, agent,
                             int(agent.get("model_file_id", 0)), tags))


def templates() -> list[Template]:
    return sorted(_REGISTRY.values(), key=lambda t: t.path.lower())


def load_errors() -> list[str]:
    return list(_ERRORS)


def _exec_file(path: Path) -> None:
    name = f"asura_cs_userlib_{abs(hash(str(path)))}"
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        return
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    try:
        spec.loader.exec_module(mod)
    except Exception:  # a broken user library must not take the editor down
        _ERRORS.append(f"{path}:\n{traceback.format_exc(limit=3)}")


def reload(extra_dirs: list[Path] | None = None) -> list[Template]:
    _REGISTRY.clear()
    _ERRORS.clear()
    from . import builtin  # noqa: F401  (registers on import)
    import importlib
    importlib.reload(builtin)
    for d in [Path.home() / ".asura_cs" / "library", *(extra_dirs or [])]:
        if d and Path(d).is_dir():
            for f in sorted(Path(d).glob("*.py")):
                _exec_file(f)
    return templates()


Factory = Callable[..., Template]
