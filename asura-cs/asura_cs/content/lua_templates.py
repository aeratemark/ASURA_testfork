"""Starting points for Lua content scripts.

The event names come from the *candidate events* in ASURA docs/30 ("A quest is accepted ... An NPC
interaction or dialogue choice occurs ... An enemy dies ... A character enters an area ... A timer
expires"). ASURA has not registered event names or a Lua API yet, so these stubs are a proposal the
editor keeps consistent across its exports; rename them in one place (``EVENTS``) when ASURA settles it.
"""

EVENTS = {
    "agent": [
        ("on_spawn", "self", "The agent appears in the instance."),
        ("on_interact", "self, character", "A character talks to / interacts with the agent."),
        ("on_dialogue_choice", "self, character, node, choice", "A dialogue button was chosen."),
        ("on_death", "self, killer", "The agent died."),
        ("on_timer", "self, name", "A timer started with world.start_timer fired."),
    ],
    "quest": [
        ("on_accept", "quest, character", "The quest was accepted."),
        ("on_objective", "quest, character, objective", "An objective advanced or completed."),
        ("on_complete", "quest, character", "All required objectives are done."),
        ("on_fail", "quest, character, reason", "The quest failed."),
    ],
    "area": [
        ("on_enter", "area, character", "A character entered the area."),
        ("on_leave", "area, character", "A character left the area."),
    ],
}

# Names a script can use. Documentation for the editor's completer and the help panel; ASURA must
# provide these (in Rust) before any of it runs.
API = {
    "world": ["spawn(agent_id, x, y, opts)", "despawn(agent)", "start_timer(owner, name, seconds)",
              "say(agent, text)", "find(placement_id)", "group(group_id)"],
    "quest": ["state(character, quest_id)", "start(character, quest_id)", "advance(character, quest_id, n)",
              "complete(character, quest_id)", "fail(character, quest_id, reason)"],
    "character": ["name", "level", "profession", "has_item(item, count)", "give_item(item, count)",
                  "give_gold(n)", "give_xp(n)", "flag(name)", "set_flag(name, value)"],
    "dialogue": ["open(agent, character, node_id)", "close(character)"],
}


def script_template(script_id: str, kind: str = "agent") -> str:
    lines = [
        f"-- {script_id}.lua",
        "--",
        "-- Event names follow the candidate events of ASURA docs/30; ASURA has not fixed its Lua API yet.",
        "-- Lua decides *what* happens; Rust performs movement, combat and pathing (docs/30).",
        "",
        "local M = {}",
        "",
    ]
    for name, args, doc in EVENTS.get(kind, EVENTS["agent"]):
        lines += [f"-- {doc}", f"function M.{name}({args})", "end", ""]
    lines.append("return M")
    return "\n".join(lines) + "\n"
