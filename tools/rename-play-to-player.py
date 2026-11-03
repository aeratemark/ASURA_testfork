#!/usr/bin/env python3
"""One-off: rename the per-connection record `Play` to `Player`, and name the map field `instance`.

`Play` was the record holding the character, its inventory and the map/agents it is playing in.
"Play" is not the noun: it is a player. The record becomes `Player`, and its getters become
methods on it, so a call reads `player.GetAgentID()`
instead of `Player::GetAgentID(&play)`. The map field becomes `instance`, because a map is an
instance - a `MapEntry` row is its description until the `Instance` object exists.

Text only: the fields, their order and every body are the ones that were there.
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / "crates/asura-world/src"


def read(name):
    return (SRC / f"{name}.rs").read_text(encoding="utf-8")


def write(name, text):
    (SRC / f"{name}.rs").write_text(text, encoding="utf-8")


def main():
    connection = read("connection")
    lines = connection.splitlines()

    # ---- lift the record and its impl out of connection.rs ---------------------------
    start = next(i for i, line in enumerate(lines) if "What one gameplay connection knows about itself" in line)
    impl_line = next(i for i, line in enumerate(lines) if line.startswith("impl Play {"))
    close = next(i for i in range(impl_line, len(lines)) if lines[i] == "}")
    record = "\n".join(lines[start:close + 1])
    lines = lines[:start] + lines[close + 1:]
    while start < len(lines) and lines[start].strip() == "":
        lines = lines[:start] + lines[start + 1:]

    # ---- the two comment lines that use "play" as a word -----------------------------
    text = "\n".join(lines)
    text = text.replace(
        "//! One gameplay connection: hello, load, play, goodbye.",
        "//! One gameplay connection: hello, load, gameplay, goodbye.",
    )
    text = text.replace(
        "// live token play as any character on the account.",
        "// live token plays as any character on the account.",
    )

    write("connection", text + "\n")

    # ---- rename the type and the locals, then the getters to methods ------------------
    for name in ("connection", "dispatch", "item", "agent", "chat", "creation", "map", "player", "travel", "world"):
        body = read(name)
        body = re.sub(r"\bPlay\b", "Player", body)
        body = re.sub(r"\bplay\b", "player", body)
        body = body.replace("player.map", "player.instance")
        body = body.replace("        map,\n", "        instance: map,\n")
        body = re.sub(r"Player::GetAgentID\(&?player\)", "player.GetAgentID()", body)
        body = re.sub(r"Player::GetName\(&?player\)", "player.GetName()", body)
        body = re.sub(r"Player::GetPlayerNumber\(&?player\)", "player.GetPlayerNumber()", body)
        body = body.replace("library does: `Player::GetAgentID(play)`.", "library does: `player.GetAgentID()`.")
        write(name, body)

    # ---- the record lands in player.rs, merged with the getters ----------------------
    player = read("player")
    record = re.sub(r"\bPlay\b", "Player", record)
    record = record.replace("    pub map: MapEntry,", "    pub instance: MapEntry,")
    record = record.replace(
        "/// What one gameplay connection knows about itself.",
        "/// The player: the character one connection drives, and the instance it is in.\n///\n"
        "/// The player as a record: the state this server owns, with the domain's verbs on it\n"
        "/// (`player.GetAgentID()`).",
    )
    record = record.replace(
        "    pub instance: MapEntry,",
        "    /// The instance this player is in. A map is an instance: this row is its description\n"
        "    /// (map id, type, spawn, capacity) until the `Instance` object exists.\n"
        "    pub instance: MapEntry,",
    )
    record = record.replace("impl Player {", "#[allow(non_snake_case)]\nimpl Player {")
    getters = '''
    /// `Player.GetAgentID()`: the agent this character drives.
    pub fn GetAgentID(&self) -> u32 {
        self.agents.player.agent_id
    }

    /// `Player.GetPlayerNumber()`: the player id the wire calls the player.
    pub fn GetPlayerNumber(&self) -> u32 {
        self.player_id
    }

    /// `Player.GetName()`: what `GAME_SMSG_PLAYER_CREATE` calls the character.
    pub fn GetName(&self) -> &str {
        &self.name
    }

    /// `Player.GetXY()`: where the character stands, as the server last believed it.
    pub fn GetXY(&self) -> (f32, f32) {
        self.agents.player.position
    }

    /// `Player.GetAgent()`: the agent record itself.
    pub fn GetAgent(&self) -> &AgentState {
        &self.agents.player
    }

    /// `Player.GetPlayerUUID()`: the character's uuid, the one the login connection carried.
    pub fn GetPlayerUUID(&self) -> Uuid {
        self.character
    }

    /// `Player.GetGoldOnCharacter()`: the gold the load block reports.
    pub fn GetGoldOnCharacter(&self) -> u32 {
        self.traits.gold
    }
'''
    assert record.rstrip().endswith("}"), "the lifted impl did not end with a brace"
    record = record.rstrip()[:-1] + getters.lstrip("\n") + "}\n"

    # replace the old namespace (unit struct + statics) with the record
    start = player.index("/// The server's `Player`")
    end = player.index("/// The one player and one agent this world server has.")
    player = player[:start] + record + "\n" + player[end:]

    write("player", player)

    # the gold getter moved from Inventory to the player it reads from
    item = read("item")
    item = item.replace(
        "    /// `Inventory.GetGoldOnCharacter()`: the gold the load block reports.\n"
        "    pub fn GetGoldOnCharacter(player: &Player) -> u32 {\n"
        "        player.traits.gold\n"
        "    }\n\n",
        "",
    )
    write("item", item)

    print("renamed Play -> Player, map -> instance; getters are methods now")


if __name__ == "__main__":
    main()
