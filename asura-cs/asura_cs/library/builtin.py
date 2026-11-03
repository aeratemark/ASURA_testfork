"""Built-in palette entries.

Markers are engine concepts and safe to ship. The agent archetypes are *editor* starting points with
no GW model id (``model_file_id = 0``): pick the real model in the property panel (or from the
"Models in this map" palette group) once you know it. Nothing here claims to be base-game data.
"""

from . import agent_template, object_template

# --- markers ------------------------------------------------------------------------------------
object_template("Markers", "Spawn point", "spawn", "Where characters arrive (map_spawn row).")
object_template("Markers", "Travel portal", "portal", "A doorway to another instance.", radius=250.0)
object_template("Markers", "Waypoint", "waypoint", "A point on a patrol/escort route.")
object_template("Markers", "Trigger area", "area", "Runs Lua when a character enters/leaves.", radius=500.0)
object_template("Markers", "Note marker", "marker", "An editor-only note.")

# --- agent archetypes -------------------------------------------------------------------------
agent_template("Agents/Town", "Villager",
               dict(name="Villager", category="npc", allegiance="npc", level=1, behaviour="wander"),
               dict(movement="roam", roam_radius=400.0))
agent_template("Agents/Town", "Guard",
               dict(name="Guard", category="npc", allegiance="ally", level=20, profession="warrior",
                    behaviour="guard"),
               dict(movement="stand"))
agent_template("Agents/Town", "Merchant",
               dict(name="Merchant", category="merchant", allegiance="npc", level=10,
                    services=["merchant"], behaviour="idle"),
               dict(movement="stand"))
agent_template("Agents/Town", "Quest giver",
               dict(name="Quest Giver", category="npc", allegiance="npc", level=10,
                    services=["quest_giver"], behaviour="idle"),
               dict(movement="stand"))
agent_template("Agents/Hostile", "Creature",
               dict(name="Creature", category="creature", allegiance="enemy", level=3,
                    behaviour="combat_default"),
               dict(movement="roam", roam_radius=300.0, lifetime="respawn", respawn_seconds=120.0))
agent_template("Agents/Hostile", "Boss",
               dict(name="Boss", category="boss", allegiance="enemy", level=20, behaviour="combat_default"),
               dict(movement="stand", lifetime="until_killed"))
