# HeroAI headless: server reuse and scaling

Source review, 2026-11-01. The owner's workload is 100 instances with 200 AI-controlled agents in
each: 20,000 independently controlled agents. The owner asked whether the client-side headless
HeroAI can supply their combat decisions after a Rust port.

## Finding

HeroAI supplies substantial reusable combat decision logic. The owner clarified that the port
uses ASURA's own state and combat systems: client support structures are outside its scope.
Their replacement is assumed, not a feasibility objection. Capacity for 20,000 agents remains
unmeasured. The small behavior tree is orchestration; skill selection, target searches, state
queries, and the work requested by those decisions are the larger body of work to examine.

The proposed port reuses the decision rules against ASURA's agent state and action API. It needs
actor-relative context, server-owned combat mechanics, and the execution mappings in
`docs/adr/0014-behavior-tree-execution.md`. This review adds no implementation or runtime policy.

## Source and limits of this review

Inspected Py4GW Reforged commit `a5fd2e54d504605200a752b1bc991d618ed33c41`:
[repository tree](https://github.com/apoguita/Py4GW_Reforged/tree/a5fd2e54d504605200a752b1bc991d618ed33c41).
Paths below are relative to that tree. All observations are from source inspection; the source
is the only witness. No source code was executed, no client capture was made, and no CPU or
memory benchmark was performed for this workload.

The review follows the headless wrapper, generic fallback combat evaluator, targeting helpers,
cache/party data, interrupt observer, and build-selection dispatch. It does not establish complete
skill coverage, validate the correctness of the combat rules, or review every profession's skill
definitions and every specialized build implementation.

## Actual decision path

| Stage | Source | Work performed |
|---|---|---|
| Headless tick | `Py4GWCoreLib/HeroAI/headless_tree.py:255`, `:260` | Processes control messages, resurrection-scroll upkeep and cache refresh, then initialization and tree execution |
| Initialization | `Py4GWCoreLib/HeroAI/headless_tree.py:231` | Checks client/map/party state, assigns cached data, and refreshes combat data before ticking the tree |
| BT orchestration | `Py4GWCoreLib/HeroAI/headless_tree.py:278` | Selects looting, out-of-combat work, following, or combat, behind guards and a casting wait |
| Build dispatch | `Py4GWCoreLib/Builds/Any/HeroAI.py:90`, `:169`, `:237` | Resolves a build from professions and skillbar, retains coroutine progress, and dispatches to that build or generic fallback |
| Generic combat | `Py4GWCoreLib/HeroAI/combat.py:1880`, `:2120` | Scans up to eight prioritized slots, chooses the first usable skill/target, or attempts auto-attack in combat |
| Cast checks | `Py4GWCoreLib/HeroAI/combat.py:1592` | Rejects cooldown, casting, adrenaline, energy, and other failures before expensive target resolution at `:1686` |
| Target and effect checks | `Py4GWCoreLib/HeroAI/combat.py:772`, `:1045`, `:1065` | Chooses suitable enemies/allies and evaluates skill-specific conditions |
| Issue action | `Py4GWCoreLib/HeroAI/combat.py:2169` | Requests skill use through the client skillbar wrapper |

Build selection is not a full registry search on every unchanged tick: a matching signature reuses
the resolved build (`Py4GWCoreLib/Builds/Any/HeroAI.py:99`). The signature itself is checked during
execution. Build discovery and per-registry runtime build instances also have caches
(`Py4GWCoreLib/BuildMgr.py:2112`, `:2173`, `:2201`). A generic-only benchmark would not measure
the specialized builds selected by the real dispatch.

The AI decides what to attempt. Calling `UseSkill` does not implement authoritative damage,
healing, effects, resource consumption, cast completion, or attack resolution. Those mechanics
remain server simulation work. The custom skill definitions include decision conditions and
coordination flags (`Py4GWCoreLib/HeroAI/custom_skill_src/skill_types.py:7`, `:103`); that is not
proof of a complete implementation of the game's skill effects.

## Assumed replacement of client support structures

These source dependencies identify where decisions obtain their inputs today. The port supplies
those inputs from the server; it does not reproduce shared memory, global caches, or account
structures. The table is a mapping aid, not a list of barriers to server feasibility.

| Assumption and evidence | Consequence for server reuse |
|---|---|
| `GameData` and `CacheData` are process singletons (`Py4GWCoreLib/HeroAI/cache_data.py:22`, `:89`) | Creating 200 wrappers would not create 200 independent actor contexts |
| Decisions repeatedly use `Player.GetAgentID`, position, target, and the local skillbar (`Py4GWCoreLib/HeroAI/combat.py:96`, `:772`, `:1592`) | Every query needs an explicit controlled agent and instance; enemy/ally relationships must be relative to that agent |
| Energy comes from a party shared-memory record and returns -1 when unavailable (`Py4GWCoreLib/HeroAI/utils.py:95`) | Ordinary server NPCs cannot depend on account slots to obtain their own energy |
| Party and effect queries use shared account records, with fallbacks for incompletely observed entities (`Py4GWCoreLib/HeroAI/party_cache.py:52`; `combat.py:1017`; `utils.py:115`) | Read authoritative state through an actor context; preserve or explicitly adapt client-visibility assumptions |
| Casting is inferred by a per-frame observer, and feasibility includes ping (`Py4GWCoreLib/HeroAI/interrupt.py:205`, `:648`, `:697`) | Server cast state can answer these queries directly, but the NPC's allowed knowledge and reaction delay are explicit behavior decisions |
| Following uses account assignments, shared-memory commands, and client controls (`Py4GWCoreLib/HeroAI/follow/follower_runtime.py:5`, `:143`) | Reuse relevant follow intent against Rust movement; account control is not an NPC movement interface |

The server's more complete information must not silently make NPCs react instantly or know more
than their intended policy allows. Replacing observation and network compensation with server
state requires a recorded timing/perception decision, especially for interrupts.

## Scaling risks found in the source

These are structural costs and benchmark candidates, not measured rankings of CPU consumption.

### Repeated skill preparation

`CacheData.Update` throttles part of the client-cache refresh with a default 75 ms interval
(`cache_data.py:144`, `:282`). `UpdateCombat` is separate: it calls `CombatClass.Update` and
`PrioritizeSkills` without that throttle (`cache_data.py:255`). Headless initialization calls it
before the tree's casting gate (`headless_tree.py:231`). Build preparation calls it again when
that coroutine path runs (`Py4GWCoreLib/Builds/Any/HeroAI.py:147`).

`PrioritizeSkills` reconstructs eight skill records, then scans them through nature, type, combo,
and remaining-slot passes (`combat.py:347`). The decision data and ordering need not be rebuilt
merely because time advanced if their inputs have not changed. A server representation can retain
an immutable skillbar plan and read current cooldown/resources separately; the invalidation inputs
and preservation of ordering still need to be established during the port.

### Repeated target scans and sorts

The skill-aware clustered-enemy helper scores each candidate by querying nearby enemies twice and
calculating cluster coordinates (`Py4GWCoreLib/HeroAI/targeting.py:320`). Each filtered-enemy query
walks the enemy array (`Py4GWCoreLib/routines_src/Agents.py:178`). If C candidates are considered
from an enemy array of E entries, those inner queries alone perform O(C * E) filtering work for
one clustered-target selection, before other scoring work. Dense cases approach quadratic work.

Weapon-spell targeting also counts nearby enemies for each eligible ally (`targeting.py:432`).
Lowest-health ally targeting constructs, filters, and sorts candidates (`targeting.py:98`).
Nearest-enemy selection sorts its filtered array (`routines_src/Agents.py:198`). These operations
are potentially repeated by many actors and multiple skill candidates.

Spatial indexing and reusable per-instance query data are candidates for the Rust implementation.
Reuse must preserve range, faction, skill-specific eligibility, ordering, and tie-breaking. A
single shared chosen target is not equivalent to per-agent targeting.

### Repeated observation and state lookup

The interrupt observer scans visible enemies every client frame (`interrupt.py:205`, `:745`).
Effect checks can rebuild effect lists and scan shared account slots (`combat.py:1017`, `:1045`).
The server already owns cast and effect state; replicating those client observation loops for each
NPC would duplicate work. Indexed access or versioned caches are approaches to evaluate, with
invalidation and reaction timing defined explicitly.

The custom skill table is already constructed once at module scope (`combat.py:31`). Its
`CustomSkillClass` allocates 3,433 entries (`custom_skill.py:19`); this is table capacity, not a
verified supported-skill count. A Rust port can likewise share immutable skill definitions while
keeping skill cooldowns, selected targets, cast progress, and behavior state separate per agent.

## What 100 instances with 200 agents means

The active-agent count is 20,000 if all 200 agents in every instance require decisions. For a
decision frequency f, the workload is 20,000 * f agent decisions per second. Frequency is not
selected here; BT decision frequency, movement integration, cast deadlines, and network updates
are different concerns.

Illustrative arithmetic at 10 decisions per second, or 200,000 decisions per second:

| Average CPU time per agent decision | Aggregate CPU-seconds required each second |
|---|---|
| 10 microseconds | 2 |
| 50 microseconds | 10 |
| 100 microseconds | 20 |

These are examples, not measurements or required rates. They exclude other server work and spare
capacity. Bursts and slow individual instance updates matter as well as averages. Serving 100
independent instances allows distributing work across cores and world processes; no capacity per
machine is established by this source review.

Keep all 200 agents active for a worst-case combat measurement. Dormant-agent scheduling is a
separate potential saving, not a reason to omit that case or freeze distant quest participants.

## Port and validation boundary

The source supports proceeding with a port of HeroAI's combat policy against server state and
actions, following the node mapping in ADR 0014. Its searches, preparation work, and scheduling
still need measurement before claiming efficient execution for 20,000 agents.

When implementation is authorized:

1. Map the selected generic and specialized build behavior to Rust state queries and actions,
   including faction, party, cast timing, and coordination scope.
2. Preserve decisions while replacing repeated client observation and preparation work; apply
   ADR 0014 to node lifecycle differences.
3. Measure one 200-agent instance with varied skillbars, dense area targeting, healing, effects,
   interrupts, and movement. Distinguish idle and all-active combat cases.
4. Measure concurrent instances, through 100 when resources permit, recording decision/update
   latency, missed deadlines, CPU, memory, path searches, and outgoing traffic.
5. Validate gameplay and the packet stream with a real client. Synthetic timing alone does not
   establish parity, complete combat behavior, or acceptable reaction timing.

No new benchmark harness or external runtime dependency is introduced by this review.
