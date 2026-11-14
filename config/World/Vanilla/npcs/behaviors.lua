-- NPC behavior trees for this world pack.
--
-- The shape lives here; the runtime and every running state live in Rust
-- (docs/adr/0014-behavior-tree-execution.md). A placement names one of these entries in its
-- "behavior" field.
--
-- Two forms are accepted:
--
--   a tree table, made from the node kinds below;
--   a function, called once per agent per tick with a `ctx` table.
--
-- Tree node kinds:
--
--   sequence  { kind = "sequence", children = { ... } }
--   fallback  { kind = "fallback", children = { ... } }
--   wait      { kind = "wait", ms = 1000 }
--   stand     { kind = "stand" }
--   patrol    { kind = "patrol", route = "route-id" }  -- route may be omitted for the placement's own
--   burrow    { kind = "burrow", visible_ms = 5000, hidden_ms = 4000 }
--
-- A function entry is `{ script = fn, burrow = true }`; `burrow` starts the body in the measured
-- 2.00 s emerge transition and lets the script own the cycle with `ctx:burrow(hidden_ms)` and
-- `ctx:emerge()`. The function gets:
--
--   ctx.agent_id, ctx.map_id, ctx.delta_ms, ctx.in_combat
--   ctx.phase        "emerging", "out", "submerging" or "hidden"
--   ctx.state        a persistent table, one per agent
--   ctx:burrow(ms)   submerge; a positive ms emerges by itself after that long, 0 waits for emerge
--   ctx:emerge()     come back early from hidden
--
-- A function returns "running" (the default), "success" (the body stands) or "failure" (it parks and
-- the reason is logged). An error parks it too, with the Lua message in the log.
return {
    -- The built-in route walk the pack's guards already use.
    patrol = { kind = "patrol" },

    -- mob-1442 in Lakeside County (map 146), the Plague Worm. The owner's live client (2026-11-14):
    -- it does not walk, it spits from its hole, it stays up while a fight drives it, and outside a
    -- fight it is burrowed most of the time. The surface and hidden spans are drawn afresh per cycle:
    -- about 5-10 s up, then 5-15 s under. The spans are content; the 2.00 s transitions are Rust's.
    worm_lakeside = {
        burrow = true,
        script = function(ctx)
            local state = ctx.state
            if ctx.in_combat then
                state.out_ms = nil
                state.out_for = nil
                if ctx.phase == "hidden" then
                    ctx:emerge()
                end
                return "running"
            end
            if ctx.phase ~= "out" then
                state.out_ms = nil
                state.out_for = nil
                return "running"
            end
            if not state.out_for then
                state.out_for = math.random(5000, 10000)
                state.out_ms = 0
            end
            state.out_ms = state.out_ms + ctx.delta_ms
            if state.out_ms >= state.out_for then
                state.out_ms = nil
                state.out_for = nil
                ctx:burrow(math.random(5000, 15000))
            end
            return "running"
        end,
    },

    -- The Worm Queen (mob-1445-type in Lakeside County, map 146): a larger Plague Worm and the quest
    -- spawn of "The Worm Problem". She comes up once and stays up - the same measured 2.00 s emerge,
    -- then the surfaced idle. Without the emerge status the client draws her buried idle, which is the
    -- animation glitch the owner reported. She does not cycle.
    worm_queen = {
        burrow = true,
        script = function(ctx)
            return "running"
        end,
    },
}
