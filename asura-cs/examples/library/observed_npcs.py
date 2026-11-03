"""Example user library: an NPC definition with *observed* client fields.

Copy this file to ``~/.asura_cs/library/`` or ``<profile>/library/`` and it appears in the Object
Window under "Observed".

The values are the one NPC row the Rurik research project reports as rendering correctly in a real
client (``content/npcs.toml``, ``[npc.hatcher]``, build 38797): model file id, composite model id,
four name string ids the client resolves to "Hatcher [Collector]", and the flags word. They are
documentation of an observed fact, kept with their provenance; ASURA's own rule is that base-game
values carry evidence (``source`` / ``source_note``). Verify against your own client before relying
on them.
"""

from asura_cs.library import agent_template

agent_template(
    "Observed", "Hatcher [Collector]",
    agent=dict(
        id="hatcher_collector", name="Hatcher [Collector]", category="npc", allegiance="npc",
        model_file_id=116228, model_id=116703, profession="monk", level=1,
        enc_name=[0x328A, 0xE3B9, 0xAA36, 0x2E69], appearance=0x64000000, npc_flags=0x20C,
        services=["collector"], behaviour="idle",
        source="observed",
        source_note="Rurik content/npcs.toml [npc.hatcher]: observed rendering on client build 38797; "
                    "file id 116228 also in gw-preservation and GWLP-R. Profession 3 on the wire.",
    ),
    placement=dict(movement="stand"),
    description="A collector NPC whose client fields were observed working (via Rurik).",
)
