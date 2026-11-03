# World content and scripting

Design discussion recorded 2026-11-01. This is a planning document, not an implemented subsystem.
The source for the decisions below is the owner's instructions in that discussion, not a claim
about the live game's internal design.

## Chosen: Rust simulation and Lua content scripts

The owner selected **Lua** for content scripting on 2026-11-01, replacing Rhai after reading both:
"Seems too new, not much documentation and examples and real world usage. I think ill do LUA
instead." Contributors should be able to edit text files and run them through ASURA without
installing a separate scripting interpreter or rebuilding the server for each content change. Lua is
intended to be embedded in the server; it has not been added to Cargo or the runtime yet.

What the change carries, recorded so it is not discovered later:

- the build gains a C toolchain. A vendored Lua compiles from C source: 50.6 s cold build and 19.8 s
  incremental in a scratch crate on 2026-11-01, where the seven member crates in `Cargo.toml`
  (`asura-auth`, `asura-world`, `asura-webgate`, `asura-protocol`, `asura-db`, `asura-fakeworld`,
  `asura-supervisor`) are all pure Rust today;
- the sandbox becomes ours to assemble: no `io`, `os`, `debug` or `package`, no bytecode loading, an
  instruction hook and a memory ceiling. Lua supplies the parts; the policy is ours;
- quests gain suspension. Coroutines let a quest wait mid-flow, so the handler shape proposed below
  is one option rather than the only one.

The owner already has behavior-tree AI in Py4GW Reforged and intends to port that AI to Rust.
BehaviorTree.CPP is the selected execution reference for the Rust node runtime; the HeroAI port
must handle the differences recorded in `docs/adr/0014-behavior-tree-execution.md`. That port has
not started. No external project becomes a runtime dependency; the dependency rule still applies.

| Rust simulation owns | Content definitions and Lua describe |
|---|---|
| Behavior-tree execution and combat mechanics | Which available behaviors agents use and when they change |
| Pathfinding, movement, following, and patrol execution | Routes, destinations, checkpoints, and encounter sequences |
| Agent lifecycle and world state | Which agents and groups appear, where, and under what conditions |
| Game mechanics, persistence operations, and packet construction | Quest objectives, dialogue, story progression, and service availability |

Scripts request game actions through a Rust API. They do not implement the combat motor, movement,
pathfinding, or wire messages. That API and its names are not defined yet. Adding Lua does not
supply mechanics that Rust has not implemented.

The packet-stream objective remains binding for reproduction of the base game. The owner's wish
to support more complex custom content is a future capability, not permission to change existing
game behavior or invent protocol messages.

## Behavior-tree class review (2026-11-01)

**Finding: the class supplies reusable behavior compositions and HeroAI migration evidence.**
BehaviorTree.CPP is the target execution reference, as decided in
`docs/adr/0014-behavior-tree-execution.md`; the Python runtime is not a specification to port
verbatim. The review below records its current rules so the later port can account for differences.
It establishes no instance scheduling, server combat mechanics, or capacity result for thousands
of instances. This is a source review, not a port or an executed simulation test.

The source inspected is Py4GW Reforged at commit
`a5fd2e54d504605200a752b1bc991d618ed33c41`, retrieved from the repository's `main` tree.
The file paths and line numbers below refer to that commit. The source is the only witness for
these execution details; no live-client behavior is inferred from them.

- [BehaviorTree.py](https://github.com/apoguita/Py4GW_Reforged/blob/a5fd2e54d504605200a752b1bc991d618ed33c41/Py4GWCoreLib/py4gwcorelib_src/BehaviorTree.py).
- [HeroAI/headless_tree.py](https://github.com/apoguita/Py4GW_Reforged/blob/a5fd2e54d504605200a752b1bc991d618ed33c41/Py4GWCoreLib/HeroAI/headless_tree.py).

### What one tick actually visits

The owner's description of walking all nodes has two distinct parts: the wrapper walks the whole
currently exposed tree to assign the blackboard, but executing the tree does not unconditionally
execute every node. The implementation, rather than its descriptive comments, gives these rules:

| Source in `Py4GWCoreLib/py4gwcorelib_src/BehaviorTree.py` | Observed behavior |
|---|---|
| `NodeState`, line 85 | Three outcomes: running, success, failure |
| `_propagate_blackboard`, line 2045; `tick`, line 2054 | Every wrapper tick seeds client context, recursively assigns the blackboard, then ticks the root |
| `SequenceNode._tick_impl`, line 608 | Resumes at the saved child index; advances through immediate successes in the same call; stops at running or failure |
| `SelectorNode._tick_impl`, line 687 | Resumes at the saved child index; advances through failures in the same call; stops at running or success |
| `ChoiceNode._tick_impl`, line 764 | Starts from the first child each call; stops at the first non-failure result |
| `ParallelNode._tick_impl`, line 1232 | Ticks children sequentially in the calling thread; stops immediately on failure; successful children are ticked again while the parent remains running |
| `WaitNode._tick_impl`, line 1460 | Polls its callback whenever reached |
| `WaitUntilNode._tick_impl`, line 1558 | Checks its timeout and throttle; skips the callback until its check interval is due |
| `SubtreeNode`, lines 1311 and 1348 | Constructs a subtree lazily, ticks its root directly, and discards the subtree on reset |

For example, a sequence containing an already-successful condition followed by a running movement
action resumes the movement action on its next tick. It does not recheck that condition until the
sequence resets. A choice reevaluates earlier priorities instead. This distinction affects combat
guards and interruption, so a port must not silently turn one node kind into the other.

One additional timing rule matters: `ActionNode._tick_impl` (line 448) returns running when its
callback first finishes, even with `aftercast_ms = 0`. It returns the saved final result on a later
tick. A port that returns success immediately changes the sequence's timing.

`HeroAI/headless_tree.py:260` calls the tree after its update and initialization checks; its tree
construction at line 278 combines alive/distance/knockdown guards, a casting check, and a selector
for looting, out-of-combat actions, following, and combat. This is evidence of the class's existing
composition, not proof that client automation leaves implement authoritative server NPC behavior.

### Fit with the planned simulation

- Rust nodes follow the selected execution reference while the HeroAI port explicitly maps its
  source control flow to those rules. Leaves query and change ASURA's own agent state. Client
  globals, player-only context seeding, shared-memory commands, ImGui, and
  client action queues do not provide the server's world API (`BehaviorTree.py:57`, `:2024`;
  `HeroAI/headless_tree.py:1`, `:130`, `:165`).
- Every agent needs separate execution state: child indices, action flags, timers, lazy subtrees,
  and its blackboard are mutable (`BehaviorTree.py:127`, `:436`, `:593`, `:672`, `:1308`, `:1555`).
  Sharing immutable definitions is a possible Rust representation; sharing these mutable node
  objects between agents would mix their progress. The representation is not selected yet.
- Rust behavior execution and Lua event handlers remain separate responsibilities. A quest can
  request an available escort or follow behavior and react to its outcome; Lua need not run the
  movement or combat tree. This is the proposed integration, not an existing API.
- The instance scheduler decides which agents run and when. The class only processes a tick when
  called (`BehaviorTree.py:2054`); neither its running state nor its wait nodes automatically park
  work until an event. Turning polling into event subscriptions would be a separate design change.

### Scaling limits to retain in the plan

The wrapper's blackboard pass visits all currently exposed nodes before selective execution. A
tree with N exposed nodes therefore has an O(N) preparation pass on every wrapper tick, even when
only one action is running (`BehaviorTree.py:2045`). The target runtime supplies node context
without reproducing that assignment walk, following ADR 0014. The Rust representation is open;
the Python class is not being changed.

The reached nodes also collect timestamps and metrics on every call (`BehaviorTree.py:149`).
Wait throttling reduces callback work, not the cost of reaching that wait. A repeater can process
many immediately completing repetitions in one call (`BehaviorTree.py:933`). These costs and
worst-case work need measuring with representative trees; a Rust translation alone establishes
no throughput figure.

Reset and interruption need a server contract: clearing node state does not itself cancel a move,
cast, or other world action already requested (`BehaviorTree.py:234`, `:488`, `:755`). Reducing tick
frequency or adding wakeups also changes reaction timing unless that behavior is preserved.
Those policies, the simulation clock used by waits, and per-agent work budgets remain open.

The review supports reuse of HeroAI's behavior intentions and compositions, with execution
differences handled through ADR 0014. It does not establish support for 1,000 simultaneous populated
instances; that needs combat, pathfinding, memory, and scheduling measurements after an
implementation exists.

## Proposed: event-driven scripting

The discussion favors short handlers that react to events and request existing Rust actions.
The owner's final wording was "possible event driven scriptting"; the event model remains a
proposal rather than a settled implementation contract.

Candidate events discussed:

- A quest is accepted, advances, completes, or fails.
- An NPC interaction or dialogue choice occurs.
- An enemy dies or a named group is defeated.
- A character enters an area or an escort reaches a checkpoint.
- A relevant item is obtained or inventory changes.
- A timer expires.

These are content concepts, not registered event names or measured live-client triggers.
Delivery order, handler scheduling, and failure behavior are open.

Example sequence, independent of API syntax:

1. Accept the escort quest and start an existing escort behavior.
2. Reach a checkpoint and spawn an ambush group.
3. Defeat the group and let the escort continue.
4. Reach the destination and complete the objective.

Rust handles the walking and fighting throughout. A companion that follows the party instead of
leading it uses a different available behavior; it does not require scripts to calculate movement.

## Quest content

The owner described ordinary quests as combinations of these requirements:

| Requirement | Content describes | Rust support needed |
|---|---|---|
| Fetch or loot items | Item identity and required count | Inventory queries and relevant events |
| Kill a boss or pack | Enemy or group identity and completion condition | Death events and group tracking |
| Reach a destination | Area or checkpoint and conditions | Area-entry or checkpoint detection |
| Escort an NPC | Route, checkpoints, success and failure conditions | Movement, behavior trees, and combat |
| Have an NPC accompany the party | Companion identity and when it follows or leaves | Following and combat behavior |
| Require earlier progress | Prerequisites and follow-up quests | Quest-state queries and updates |

Standard objectives can be combined into sequences, requirements that all must be met, or
alternative ways to succeed. Lua provides room for special branches and sequences beyond those
ordinary combinations. A new sequence of existing actions is distinct from a new engine mechanic.

Main quests, secondary quests, and quest chains share these concepts. An NPC's existence, movement,
and available quests are separate: a roaming NPC can offer a quest, and a permanent villager can
offer different quests according to progress.

Whether ordinary objectives are authored as data, Lua, or both is not decided. JSON was discussed
as a possible data format, not selected as the storage format. A map-canvas editor and state
graphs were discussed as authoring approaches, not approved implementation work. The initial
map/portal editing screen is part of the handoff in `docs/32-world-simulation-plan.md`.

## Map spawns and layouts

The content design discussed separates the following concepts. These are planning terms, not
existing schema tables or Rust types.

| Concept | Meaning |
|---|---|
| Agent definition | Model, faction, stats, available services, and assigned behavior |
| Placement | Position and facing, or a permitted area in which to spawn |
| Group | Agents belonging to a pack, patrol, reinforcement wave, or ambush |
| Movement assignment | Standing, roaming within an area, or following a route |
| Spawn conditions | Always present, quest-dependent, or created after an event |
| Lifetime | Despawn conditions, whether death persists, and any respawn delay |
| Quest relationships | Quests an NPC offers or accepts, subject to availability conditions |

Examples covered: a map's normal enemy population, roaming service NPCs, side-quest givers,
quest companions, checkpoint ambushes, and reinforcements after a boss dies.

Layouts can be authored as exact placements, spawn areas, or named population sets such as a
peaceful village and an occupied village. These are available approaches, not a decision to use
random placement or replace measured base-game layouts.

Phasing means changing the population or visibility according to progress. Layering means separate
populations of a location. Neither follows automatically from selecting Lua. Character, party,
and instance state have different scopes; the population-selection policy is not decided.

## Open questions

- Do ordinary quests use objective definitions, Lua handlers, or both?
- Does possessing an item satisfy an objective, or must it be obtained during the quest?
- Does an explorable population follow the leader, combined party quests, or a per-quest rule?
- Which progress survives logout, and which belongs only to the running encounter or instance?
- How are separate instances, phases, and layers identified and selected?
- What game actions and state queries does the scripting API expose?
- How are events ordered, handlers scheduled, and duplicate triggers handled?
- What happens after a script error or execution-limit failure, including actions already applied?
- What execution, data-size, and access limits apply to scripts and Rust callbacks?
- When does changed content take effect, and what happens to active quests during reload?
- Which runtime version, Cargo features, content format, and packaging are used?

Script execution cost is now measured for one workload: 75 ns per decision in Lua 5.4 against
1,027 ns in Rhai on the same loop, release build, 2026-11-01. Memory per instance, debugger
usability, Windows/Linux packaging effort, and reload behavior are still unmeasured, and this
document makes no claim about them.

## Current code and reading

- `crates/asura-world/src/instance.rs`: the current instance, player slots, movement tick, and
  registry keyed by map id. It does not establish the future multi-instance or layering policy.
- `crates/asura-world/Cargo.toml`: no scripting runtime dependency at the time of this discussion.
- `docs/22-world-model.md`: game nouns and the owner's existing world-model decisions.
- `docs/02-architecture.md`: service ownership and the content database's architectural role.
- `docs/adr/0014-behavior-tree-execution.md`: selected execution reference and HeroAI node mappings.
- `docs/31-heroai-server-feasibility.md`: headless combat policy, client assumptions, and the
  100-instance / 200-agent scaling review.
- `docs/32-world-simulation-plan.md`: detailed implementation handoff, owner intent, the early
  map/portal manager, seven phases, acceptance checks, and questions to ask before dependent work.
- `docs/33-control-panel-design.md`: the expanded panel for server operations, capture sessions,
  guided tests, recordings, and configuration/content management (owner, 2026-11-02).
- [mlua, the Lua binding for Rust](https://docs.rs/mlua/latest/mlua/).
- [The Lua 5.4 reference manual](https://www.lua.org/manual/5.4/): a debug hook counts instructions,
  and an instruction count does not bound the time spent inside a Rust function.

This records the selected language and the design under discussion. It does not authorize a
runtime integration, AI port, schema migration, content conversion, or change to live behavior.
