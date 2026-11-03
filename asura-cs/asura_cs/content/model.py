"""The content model: what a profile contains.

Follows the planning vocabulary of ASURA ``docs/30-world-content-and-scripting.md`` and the profile
rules of ``docs/34-self-contained-profiles.md``:

* **Agent definition** - model, allegiance, level, services, behaviour, dialogue, script.
* **Placement** - an agent put somewhere in a map, with facing, group, movement and spawn rules.
* **Group / Route / Area** - packs, patrol routes (ordered waypoints), trigger areas.
* **Spawn point / Travel portal** - where characters arrive, and doorways with their destination
  and landing point (the simulator plan's Phase 1 content).
* **Dialogue** - nodes of NPC text with player choices; each node/choice has a Lua *condition* and a
  Lua *result*, in the spirit of the TES Construction Set's Info conditions and Result box.
* **Quest** - objectives, prerequisites, rewards, giver.
* **Script** - a Lua file; objects reference scripts by id.

ASURA has not fixed a content storage format or the Lua event contract yet; this model is the
editor's working form and exporters translate it (see ``asura_cs.export``).

Every editable attribute carries metadata (``meta(...)``) so the property panel can build the right
widget without per-class UI code.
"""

from __future__ import annotations

import itertools
import math
import re
from dataclasses import dataclass, field, fields
from typing import Any, ClassVar

# ----------------------------------------------------------------------------------------------
# field metadata


def meta(label: str = "", kind: str = "str", *, choices: tuple = (), ref: str = "", group: str = "General",
         tip: str = "", readonly: bool = False, hidden: bool = False, default: Any = None,
         default_factory=None):
    """A dataclass field with editor metadata.

    kind: str | text | lua | int | float | bool | enum | ref | angle | model | list | words
    ref:  for kind == "ref": the record type the value names ("agent", "dialogue", "script", ...).
    """
    md = {"label": label, "kind": kind, "choices": choices, "ref": ref, "group": group, "tip": tip,
          "readonly": readonly, "hidden": hidden}
    if default_factory is not None:
        return field(default_factory=default_factory, metadata=md)
    return field(default=default, metadata=md)


def editable_fields(obj) -> list:
    return [f for f in fields(obj) if f.metadata and not f.metadata.get("hidden")]


_ID_OK = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")


def valid_id(s: str) -> bool:
    return bool(_ID_OK.match(s or ""))


def slug(text: str) -> str:
    s = re.sub(r"[^A-Za-z0-9]+", "_", text).strip("_").lower()
    if not s or s[0].isdigit():
        s = "id_" + s
    return s


# ----------------------------------------------------------------------------------------------
# records


@dataclass(eq=False)
class Record:
    """Anything with an editor id. ``eq=False``: records are compared by identity."""
    TYPE: ClassVar[str] = "record"
    TYPE_LABEL: ClassVar[str] = "Record"

    id: str = meta("Editor ID", "str", group="General", default="",
                   tip="Unique id within the profile; other content refers to this record by it.")

    def display_name(self) -> str:
        return getattr(self, "name", "") or self.id


ALLEGIANCES = ("npc", "ally", "neutral", "enemy", "spirit_pet", "minion")
# Index in this tuple = the profession number the client uses (1 warrior ... 10 dervish).
PROFESSIONS = ("none", "warrior", "ranger", "monk", "necromancer", "mesmer", "elementalist",
               "assassin", "ritualist", "paragon", "dervish")
SERVICES = ("merchant", "trader", "collector", "skill_trainer", "crafter", "storage", "guild_registrar",
            "priest", "henchman", "quest_giver")
BEHAVIOURS = ("idle", "wander", "patrol", "guard", "follow_party", "escort", "combat_default")
MOVEMENT = ("stand", "roam", "route", "follow")
LIFETIME = ("permanent", "until_killed", "respawn", "until_event")
SOURCES = ("invented", "observed", "capture", "wiki", "gw-dat")

# Dialogue option kinds. The GW client draws the option icon from a kind byte (GAME_SMSG 0x007E);
# the quest kinds and their icons are Rurik's on-screen measurements (toolkit/authsrv/questdefs.py).
# "talk" is an editor-level plain choice with no measured wire kind yet.
CHOICE_KINDS = ("talk", "quest_available", "accept", "decline", "advance", "in_progress", "turn_in")
CHOICE_WIRE_KIND = {"quest_available": 18, "accept": 16, "decline": 17, "advance": 21, "in_progress": 22,
                    "turn_in": 23}
DIALOG_LINE_UNITS = 122   # 0x0080 body line, UTF-16 code units (Rurik DIALOG_UNITS)
FIELD_UNITS = 128         # option labels and quest description/objectives (Rurik FIELD_UNITS)


@dataclass(eq=False)
class AgentDef(Record):
    """An agent definition (TES: a base object; GW: an NPC/creature template)."""
    TYPE: ClassVar[str] = "agent"
    TYPE_LABEL: ClassVar[str] = "Agent definition"

    name: str = meta("Name", default="New Agent")
    category: str = meta("Category", "enum", choices=("npc", "creature", "merchant", "boss", "henchman",
                                                      "critter", "gadget"), default="npc")
    model_file_id: int = meta("Model file id", "model", group="Appearance", default=0,
                              tip="Gw.dat file id of the agent model (0 = unknown, shown as a marker).")
    allegiance: str = meta("Allegiance", "enum", choices=ALLEGIANCES, group="Stats", default="npc")
    level: int = meta("Level", "int", group="Stats", default=1)
    profession: str = meta("Profession", "enum", choices=PROFESSIONS, group="Stats", default="none")
    faction: str = meta("Faction", group="Stats", default="")
    services: list = meta("Services", "list", choices=SERVICES, group="Services", default_factory=list)
    behaviour: str = meta("Behaviour", "enum", choices=BEHAVIOURS, group="Behaviour", default="idle",
                          tip="Name of a Rust behaviour; Lua chooses, Rust executes (docs/30).")
    dialogue: str = meta("Dialogue", "ref", ref="dialogue", group="Behaviour", default="")
    script: str = meta("Script", "ref", ref="script", group="Behaviour", default="")
    # What the GW client must be told before an agent of this definition can exist. Field set as
    # documented by Rurik (content/npcs.toml): the client renders the name from string ids, not text.
    model_id: int = meta("Composite model id", "model", group="Client", default=0,
                         tip="Second model id some NPCs carry (Rurik: absent for creatures sent without "
                             "a composite). 0 = none.")
    enc_name: list = meta("Name string ids", "words", group="Client", default_factory=list,
                          tip="GW string ids (hex words) the client resolves to the nameplate in its own "
                              "language. Copied from observation, not invented. Empty = no nameplate.")
    appearance: int = meta("Appearance word", "model", group="Client", default=0x64000000,
                           tip="Packed hue/saturation/lightness/scale; 0x64000000 = unmodified, 100% scale.")
    npc_flags: int = meta("NPC flags", "model", group="Client", default=0,
                          tip="Client agent flags word (e.g. 0x20C observed on a collector).")
    speed: float = meta("Move speed", "float", group="Client", default=0.0, tip="0 = engine default")
    source: str = meta("Source", "enum", choices=SOURCES, group="Provenance", default="invented",
                       tip="Where the values come from: ASURA requires evidence for base-game content.")
    source_note: str = meta("Evidence", "text", group="Provenance", default="")
    notes: str = meta("Notes", "text", group="Notes", default="")


@dataclass(eq=False)
class Dialogue(Record):
    TYPE: ClassVar[str] = "dialogue"
    TYPE_LABEL: ClassVar[str] = "Dialogue"

    name: str = meta("Name", default="New Dialogue")
    start: str = meta("Start node", "str", default="greeting",
                      tip="Node shown when the conversation opens; the first node whose condition passes "
                          "among start and its alternatives is used.")
    nodes: list = meta("Nodes", "list", hidden=True, default_factory=list)


@dataclass(eq=False)
class DialogueChoice:
    text: str = ""
    goto: str = ""              # node id, or "" / "end" to close the dialogue
    condition: str = ""         # Lua expression; empty = always shown
    result: str = ""            # Lua statements run when picked
    kind: str = "talk"          # one of CHOICE_KINDS (decides the icon in the client)
    quest: str = ""             # quest id for the quest kinds


@dataclass(eq=False)
class DialogueNode:
    id: str = "greeting"
    text: str = ""
    condition: str = ""         # Lua expression gating this node
    result: str = ""            # Lua run when the node is shown
    choices: list = field(default_factory=list)
    alternatives: list = field(default_factory=list)  # other node ids tried in order if condition fails


OBJECTIVE_KINDS = ("talk", "fetch", "kill", "reach", "escort", "accompany", "custom")


@dataclass(eq=False)
class Objective:
    kind: str = "talk"
    target: str = ""            # agent/placement/area/item id depending on kind
    count: int = 1
    description: str = ""
    optional: bool = False


@dataclass(eq=False)
class Quest(Record):
    TYPE: ClassVar[str] = "quest"
    TYPE_LABEL: ClassVar[str] = "Quest"

    name: str = meta("Name", default="New Quest")
    giver: str = meta("Quest giver", "ref", ref="agent", default="")
    description: str = meta("Log text", "text", default="", tip="Quest log description (max 128 units on the wire).")
    objectives_text: str = meta("Objectives line", "text", default="")
    offer_dialogue: str = meta("Offer", "text", group="Dialogue", default="",
                               tip="What the giver says while the quest is on offer.")
    in_progress_dialogue: str = meta("In progress", "text", group="Dialogue", default="")
    objective_dialogue: str = meta("Objective NPC line", "text", group="Dialogue", default="")
    turn_in_dialogue: str = meta("Turn in", "text", group="Dialogue", default="")
    quest_id: int = meta("Client quest id", "int", group="Client", default=0,
                         tip="The client's challenge id. Rurik found the client bounds it (1465 on build "
                             "38797), leaving few free ids for new quests. 0 = unassigned.")
    enc_name: list = meta("Name string ids", "words", group="Client", default_factory=list)
    enc_region: list = meta("Region string ids", "words", group="Client", default_factory=list)
    log_flags: int = meta("Log flags", "int", group="Client", default=32, tip="32 = 'Primary Quests'.")
    prerequisites: list = meta("Prerequisites", "list", ref="quest", default_factory=list)
    objectives: list = meta("Objectives", "list", hidden=True, default_factory=list)
    reward_xp: int = meta("XP", "int", group="Rewards", default=0)
    reward_gold: int = meta("Gold", "int", group="Rewards", default=0)
    reward_items: str = meta("Items", "str", group="Rewards", default="")
    script: str = meta("Script", "ref", ref="script", group="Script", default="")
    source: str = meta("Source", "enum", choices=SOURCES, group="Provenance", default="invented")
    source_note: str = meta("Evidence", "text", group="Provenance", default="")


@dataclass(eq=False)
class Script(Record):
    TYPE: ClassVar[str] = "script"
    TYPE_LABEL: ClassVar[str] = "Script"

    source: str = meta("Source", "lua", hidden=True, default="")


# ----------------------------------------------------------------------------------------------
# map objects


@dataclass(eq=False)
class MapObject(Record):
    """Something with a position in a map. GW world units; z negative is up."""
    TYPE: ClassVar[str] = "object"
    TYPE_LABEL: ClassVar[str] = "Map object"
    ICON: ClassVar[str] = "o"
    COLOR: ClassVar[str] = "#c0c0c0"

    name: str = meta("Name", default="")
    x: float = meta("X", "float", group="Placement", default=0.0)
    y: float = meta("Y", "float", group="Placement", default=0.0)
    z: float = meta("Z (neg. = up)", "float", group="Placement", default=0.0)
    plane: int = meta("Plane", "int", group="Placement", default=0)
    yaw: float = meta("Facing (deg)", "angle", group="Placement", default=0.0,
                      tip="Degrees from +X (east) towards +Y (north).")
    layer: str = meta("Layer", group="Placement", default="",
                      tip="Free-form layer name for grouping/filtering (e.g. 'peaceful', 'occupied').")

    @property
    def pos(self) -> tuple[float, float, float]:
        return self.x, self.y, self.z

    def facing_vector(self) -> tuple[float, float]:
        a = math.radians(self.yaw)
        return math.cos(a), math.sin(a)


@dataclass(eq=False)
class AgentPlacement(MapObject):
    TYPE: ClassVar[str] = "placement"
    TYPE_LABEL: ClassVar[str] = "Agent"
    ICON: ClassVar[str] = "A"
    COLOR: ClassVar[str] = "#4fc3f7"

    agent: str = meta("Agent definition", "ref", ref="agent", group="Agent", default="")
    group_id: str = meta("Group", "ref", ref="group", group="Agent", default="")
    movement: str = meta("Movement", "enum", choices=MOVEMENT, group="Movement", default="stand")
    roam_radius: float = meta("Roam radius", "float", group="Movement", default=0.0)
    route: str = meta("Route", "ref", ref="route", group="Movement", default="")
    spawn_condition: str = meta("Spawn condition", "lua", group="Spawning", default="",
                                tip="Lua expression; empty = always present.")
    lifetime: str = meta("Lifetime", "enum", choices=LIFETIME, group="Spawning", default="permanent")
    respawn_seconds: float = meta("Respawn (s)", "float", group="Spawning", default=0.0)
    script: str = meta("Script override", "ref", ref="script", group="Script", default="")


@dataclass(eq=False)
class SpawnPoint(MapObject):
    """Where a character arrives on entering the map (ASURA ``map_spawn`` row)."""
    TYPE: ClassVar[str] = "spawn"
    TYPE_LABEL: ClassVar[str] = "Spawn point"
    ICON: ClassVar[str] = "S"
    COLOR: ClassVar[str] = "#81c784"

    index: int = meta("Index", "int", group="Spawn", default=0,
                      tip="Row index in map_spawn; index 0 is the default arrival.")


@dataclass(eq=False)
class TravelPortal(MapObject):
    TYPE: ClassVar[str] = "portal"
    TYPE_LABEL: ClassVar[str] = "Travel portal"
    ICON: ClassVar[str] = "P"
    COLOR: ClassVar[str] = "#ba68c8"

    model_file_id: int = meta("Portal model", "model", group="Portal", default=0)
    radius: float = meta("Reach radius", "float", group="Portal", default=250.0)
    to_map: int = meta("Destination map", "map", group="Portal", default=0, tip="0 = no destination yet")
    arrive_x: float = meta("Arrive X", "float", group="Portal", default=0.0)
    arrive_y: float = meta("Arrive Y", "float", group="Portal", default=0.0)
    measured: bool = meta("Measured", "bool", group="Portal", default=False, readonly=True,
                          tip="Destination comes from a recorded live crossing.")
    source: str = meta("Source", "str", group="Portal", default="", readonly=True,
                       tip="Where ASURA's map_portal row came from (measured crossing, portal prop, derived, "
                           "editor).")


@dataclass(eq=False)
class Waypoint(MapObject):
    TYPE: ClassVar[str] = "waypoint"
    TYPE_LABEL: ClassVar[str] = "Waypoint"
    ICON: ClassVar[str] = "w"
    COLOR: ClassVar[str] = "#ffb74d"

    route: str = meta("Route", "ref", ref="route", group="Route", default="")
    order: int = meta("Order", "int", group="Route", default=0)
    wait_seconds: float = meta("Wait (s)", "float", group="Route", default=0.0)


@dataclass(eq=False)
class Area(MapObject):
    TYPE: ClassVar[str] = "area"
    TYPE_LABEL: ClassVar[str] = "Trigger area"
    ICON: ClassVar[str] = "T"
    COLOR: ClassVar[str] = "#e57373"

    radius: float = meta("Radius", "float", group="Area", default=500.0)
    on_enter: str = meta("On enter (Lua)", "lua", group="Area", default="")
    on_leave: str = meta("On leave (Lua)", "lua", group="Area", default="")
    once: bool = meta("Fire once", "bool", group="Area", default=False)
    script: str = meta("Script", "ref", ref="script", group="Area", default="")


@dataclass(eq=False)
class Marker(MapObject):
    TYPE: ClassVar[str] = "marker"
    TYPE_LABEL: ClassVar[str] = "Marker"
    ICON: ClassVar[str] = "x"
    COLOR: ClassVar[str] = "#fff176"

    note: str = meta("Note", "text", group="Marker", default="")


@dataclass(eq=False)
class Route(Record):
    TYPE: ClassVar[str] = "route"
    TYPE_LABEL: ClassVar[str] = "Route"
    name: str = meta("Name", default="")
    loop: bool = meta("Loop", "bool", default=True)
    mode: str = meta("Mode", "enum", choices=("patrol", "escort", "path"), default="patrol")


@dataclass(eq=False)
class Group(Record):
    TYPE: ClassVar[str] = "group"
    TYPE_LABEL: ClassVar[str] = "Group"
    name: str = meta("Name", default="")
    kind: str = meta("Kind", "enum", choices=("pack", "patrol", "wave", "ambush", "villagers"), default="pack")
    spawn_condition: str = meta("Spawn condition", "lua", default="")


OBJECT_TYPES: dict[str, type[MapObject]] = {c.TYPE: c for c in
                                            (AgentPlacement, SpawnPoint, TravelPortal, Waypoint, Area, Marker)}
RECORD_TYPES: dict[str, type[Record]] = {c.TYPE: c for c in (AgentDef, Dialogue, Quest, Script, Route, Group)}


# ----------------------------------------------------------------------------------------------
# containers


@dataclass(eq=False)
class MapContent:
    """Everything a profile says about one map (instance)."""
    map_id: int
    name: str = ""
    map_type: int = 1
    map_file_id: int = 0
    objects: list = field(default_factory=list)   # MapObject
    routes: list = field(default_factory=list)    # Route
    groups: list = field(default_factory=list)    # Group

    def label(self) -> str:
        return f"{self.name} ({self.map_id})" if self.name else f"map {self.map_id}"

    def spawns(self) -> list[SpawnPoint]:
        return sorted((o for o in self.objects if isinstance(o, SpawnPoint)), key=lambda s: s.index)

    def waypoints(self, route_id: str) -> list[Waypoint]:
        return sorted((o for o in self.objects if isinstance(o, Waypoint) and o.route == route_id),
                      key=lambda w: w.order)


@dataclass(eq=False)
class Profile:
    """A self-contained ASURA content profile (docs/34)."""
    id: str = "custom"
    name: str = "Custom profile"
    description: str = ""
    based_on: str = ""
    maps: dict = field(default_factory=dict)       # map_id -> MapContent
    agents: list = field(default_factory=list)     # AgentDef
    dialogues: list = field(default_factory=list)  # Dialogue
    quests: list = field(default_factory=list)     # Quest
    scripts: list = field(default_factory=list)    # Script

    # -- lookups ------------------------------------------------------------------------------
    def collection(self, type_name: str, map_id: int | None = None) -> list:
        if type_name == "agent":
            return self.agents
        if type_name == "dialogue":
            return self.dialogues
        if type_name == "quest":
            return self.quests
        if type_name == "script":
            return self.scripts
        if map_id is None:
            raise KeyError(f"{type_name} records live in a map; pass map_id")
        m = self.maps[map_id]
        if type_name == "route":
            return m.routes
        if type_name == "group":
            return m.groups
        return m.objects

    def find(self, type_name: str, rid: str, map_id: int | None = None):
        try:
            coll = self.collection(type_name, map_id)
        except KeyError:
            return None
        for r in coll:
            if r.id == rid and (type_name not in OBJECT_TYPES or r.TYPE == type_name):
                return r
        return None

    def all_ids(self) -> set[str]:
        ids = {r.id for coll in (self.agents, self.dialogues, self.quests, self.scripts) for r in coll}
        for m in self.maps.values():
            ids.update(o.id for o in itertools.chain(m.objects, m.routes, m.groups))
        return ids

    def unique_id(self, base: str, also_taken: set | None = None) -> str:
        base = slug(base) if not valid_id(base) else base
        taken = self.all_ids() | (also_taken or set())
        if base not in taken:
            return base
        stem = re.sub(r"_\d+$", "", base)
        for n in itertools.count(1):
            cand = f"{stem}_{n:02d}"
            if cand not in taken:
                return cand
        raise AssertionError  # pragma: no cover

    def map_of(self, obj) -> MapContent | None:
        for m in self.maps.values():
            if obj in m.objects or obj in m.routes or obj in m.groups:
                return m
        return None
