"""Which part of the game this editor works on: pre-Searing Ascalon only.

Every map list, palette entry and reference the editor offers is limited to these instances. The ids and
names are ASURA's own (``tools/map-config.sql`` and the measured crossings in ``portals.rs``).
"""

from __future__ import annotations

PRE_SEARING: dict[int, str] = {
    148: "Ascalon City",
    164: "Ashford Abbey",
    165: "Foible's Fair",
    166: "Fort Ranik",
    163: "The Barradin Estate",
    146: "Lakeside County",
    160: "Green Hills County",
    161: "Wizard's Folly",
    162: "Regent Valley",
    147: "The Northlands",
    145: "The Catacombs",
}

SCOPE_NAME = "pre-Searing Ascalon"


def in_scope(map_id: int) -> bool:
    return int(map_id) in PRE_SEARING
