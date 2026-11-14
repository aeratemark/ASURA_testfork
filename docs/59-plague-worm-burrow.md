# Plague Worm burrow

The Plague Worm is the first scripted NPC behavior: a body that is created, stands out, submerges,
is removed, and re-appears at the same id and the same coordinates. The wire and the two 2.00 s
transitions are Rust mechanics. Which body, when, and how long it stays out or hidden is content,
declared in `config/World/Vanilla/npcs/behaviors.lua` (`docs/adr/0014-behavior-tree-execution.md`).

## What Rurik measured

The evidence is Rurik's Lakeside tape, `studies/tape/FINDINGS.md` T3, and the targeted status
experiment, `studies/cmsg/FINDINGS.md` T16. The recording is
`vault/captures/live/20260807T143055` there; this repository holds no copy of it. The numbers
below are Rurik's measurement, not a re-measurement by this server.

- 140 worm re-creations across 13 worm ids, one burst shape and no exceptions.
- Every re-create is five messages, in this order:
  `0x009F[66, agent, 0]`, `0x00F0[agent, 0x1000]`, `0x0020`, `0x006D[agent, item, 0]`,
  `0x0026[agent, 9]`.
- The NPC definition (`0x0056`, and `0x0057` when the definition has model files) is sent once
  for those 140 creates. A re-create does not resend it.
- The emerge animation ends with `0x00F1[agent, 0]` exactly 2.00 s after the create. Every sample
  is inside +/-60 ms.
- The submerge animation starts with `0x00F1[agent, 0x1000]` exactly 2.00 s before the remove.
- The removal itself is a bare `0x0021` (134 of 134).
- Every worm re-emerges at byte-identical coordinates. A submerged worm does not move.
- The 0x1000 bit marks the two transition animations, not "hidden". Status 0 is surfaced.
- Death is not a burrow: a killed worm goes to the dead bit while surfaced, with no 0x1000 and no
  remove.

The two 2.00 s windows are the client's animation. They are the only fixed times in the cycle.

## What is content

Rurik measured two period families sharing one model: out 0.48-7.48 s and hidden 1.6-6.9 s for
one, and cycles of about 8-10 s and about 33 s for the other. Player distance did not explain the
split. There is no single period to copy.

The owner's live client (2026-11-14) is more specific: a Plague Worm stays surfaced while a fight
drives it, and outside a fight it is burrowed most of the time - about 5-10 s up, then 5-15 s under,
drawn afresh each cycle. The owner's observation outranks the older measurement. The owner also
named two defects in the first script: it let the worm walk, and its worm repeated the burrow
animation during a fight. The live worm stays on its hole, spits from range, and stays up while the
fight drives it.

`behaviors.lua` therefore owns the visible and hidden spans, and draws them per cycle. They are not
a measurement of this placement; a capture of the live client replaces them when one exists. The
body itself is `stationary` in its definition's combat block: the fight brain never walks it, and a
player who leaves its aggro range ends the fight where the worm stands. Its `aggression` is
`when_provoked`: GWW "Plague Worm" says they "will by default not attack", so a worm that nobody hits
keeps its burrow cycle and only the one that was damaged comes up and fights.

The Worm Queen (`mob-1445-type`) is a larger Plague Worm and the quest spawn of "The Worm Problem".
She comes up once on her create and stays surfaced: `worm_queen` is `burrow = true` with a script
that never asks to submerge, so the client gets the measured emerge and then the surfaced idle. The
pack still seeds her statically; the quest spawn itself is not implemented.

## The script form

A behavior entry may be a table (a tree, as before) or a function. The worm is a function: the
runtime calls it once per agent per tick with a context table, and it reads and writes `ctx.state`,
a table Rust creates once per agent:

```lua
worm_lakeside = {
    burrow = true,
    script = function(ctx)
        if ctx.in_combat then
            ctx.state.out_ms = nil
            ctx.state.out_for = nil
            if ctx.phase == "hidden" then ctx:emerge() end
            return "running"
        end
        if ctx.phase ~= "out" then
            ctx.state.out_ms = nil
            ctx.state.out_for = nil
            return "running"
        end
        if not ctx.state.out_for then
            ctx.state.out_for = math.random(5000, 10000)
            ctx.state.out_ms = 0
        end
        ctx.state.out_ms = ctx.state.out_ms + ctx.delta_ms
        if ctx.state.out_ms >= ctx.state.out_for then
            ctx.state.out_ms = nil
            ctx.state.out_for = nil
            ctx:burrow(math.random(5000, 15000))
        end
        return "running"
    end,
}
```

`ctx.phase` is the table's own phase (`emerging`, `out`, `submerging`, `hidden`); `ctx.in_combat` is
the fight driving the body; `ctx.delta_ms` is the tick delta, and the script accumulates its own spans
in `ctx.state`; `ctx:burrow(ms)` asks for the 2.00 s submerge and a hidden span of `ms`
(0 waits for `ctx:emerge()`), and `ctx:emerge()` comes back early. The commands become a
`BurrowRequest` the table applies through the same measured wire. The function returns `"running"`,
`"success"` or `"failure"`; an error parks the body and logs the Lua message.

`burrow = true` starts the body in the emerge transition and gives it the manual burrow state. A
script leaf on a body that does not burrow omits the flag and gets no burrow phase.

## The split in the code

| piece | where | why |
|---|---|---|
| Lua loader, tree conversion and script leaves | `crates/asura-world/src/npc/lua.rs` | a table becomes a `Spec`; a function stays a live entry and is called each tick with a Rust-owned `ctx.state` |
| `Spec::Burrow`/`Spec::Script`, `Kind::Script` and `Tree::burrow_terms` | `crates/asura-world/src/npc/behavior.rs` | the runtime keeps the tree running; the terms are content; `ctx:burrow()`/`ctx:emerge()` become a `BurrowRequest` |
| phase clock, status, remove, re-create | `crates/asura-world/src/npc/table.rs` | wire order and running state are Rust's |
| 0x1000, `entry_with_status`, `burrow_recreate` | `crates/asura-world/src/npc/burst.rs` | the measured bytes |
| which placements run it | `config/World/Vanilla/maps/.../146/npcs.json` `behavior` | the pack names the script |

The body starts in the `Emerging` phase: its first create carries `0x00F0 = 0x1000`, so the
client draws the emerge animation before the body is targetable. The table then runs
`Out -> Submerging -> Hidden -> Emerging`, sends the measured messages, and uses the five-message
burst for a viewer that was already told the body once. A viewer that arrives during a transition
gets the ordinary entry with the current status word.

## Acceptance

The live client is the judge. In Lakeside County (map 146), a worm should:

1. appear with the emerge animation, then become targetable;
2. keep the same agent id and the same coordinates after a burrow;
3. submerge for 2.00 s before it leaves the client's view;
4. stay gone for the script's hidden span;
5. re-appear without a second definition; a client that loaded the map still draws it.

The offline checks are `npc::table::tests::a_burrower_cycles_with_the_measured_transitions_and_no_definition_resend`,
`npc::lua::tests::the_committed_script_declares_the_worm_cycle` and
`npc::lua::tests::the_committed_worm_stays_up_in_combat_and_burrows_for_a_drawn_span`.
