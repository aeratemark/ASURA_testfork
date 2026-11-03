# World simulation implementation handoff

Recorded 2026-11-01. This replaces the earlier high-level proposal in this file.
It is the detailed handoff requested by the owner for another implementation agent.
It documents intended work; writing this handoff does not implement any phase.

## Read this before acting

Read AGENTS.md first. It governs the work. Then read docs/HANDOFF.md for existing decisions and
the relevant source files named below. This document does not override those rules.

The simulator's first assignment is Phase 1: configurable map spawns and portal arrivals.
The owner expanded its editing interface into a control panel for server operations, Py4GW
Stealth capture sessions and in-game data reading, guided tests, and configuration/content
management (2026-11-02). Read
docs/33-control-panel-design.md for that panel's design and unresolved product choices.
The map editor described here is the panel's content increment, not its complete product scope.
Phases 2-7 describe the subsequent simulator work and its boundaries.
Start only the phase assigned by the owner. An instruction to implement a phase authorizes the
ordinary edits, builds, and targeted checks needed for that phase; it is not permission to add
unlisted features or change unrelated gameplay.

The seven phases here are simulation work phases. They are not the older infrastructure
milestones or the four packet-parity stages described elsewhere in the repository.

If the owner assigns the whole roadmap, follow the order and the stated decision gates. Do not
skip a gate, count a placeholder as completion, or silently choose a missing product/game rule.
Ask about unresolved requirements before doing the work that depends on the answer. Continue
independent authorized work where possible. Do not ask the owner to choose variable names,
routine local code organization, or other implementation details already settled by the tree.

## The owner's intention

The owner wants an actual Guild Wars world simulation built on ASURA's working connections,
travel, movement, pathfinding, inventory, database, and packet handling. Reuse those components.
Existing data readers and configuration must reach the running world instead of remaining
disconnected from it. Content changes must not require editing and rebuilding Rust each time.

The immediate example is map travel: default map spawns, portal destinations, and the landing
points used after a crossing must be editable as content. The first visible result is editing
the Ascalon City / Lakeside County configuration through a small manager and verifying the
result with the real client.

The eventual world contains NPC placements, roaming and patrolling agents, quest givers,
services, enemy groups, combat, main and secondary quests, chains, escorts, companions, and
story-dependent encounters. These are the direction of the roadmap, not a demand to implement
all of them in Phase 1.

The AI work is a port of HeroAI's decision logic to authoritative Rust state and actions.
It is not a port of client support structures: no shared memory, GLOBAL_CACHE, account slots,
client input queues, or injected client environment. Their replacement is assumed, not a
feasibility objection to the combat policy.

The owner's sizing example is 100 simultaneous instances with 200 AI-controlled agents each:
20,000 agents. That is a workload to measure, not a claim that one machine already supports it,
not an AI decision rate, and not permission to reduce gameplay fidelity.

The packet-stream objective remains the specification for reproducing the base game. Future
custom content is intended to be possible; that does not authorize changing base-game behavior.
When a requirement or feature choice is genuinely unresolved, ask instead of designing it on
the owner's behalf. This is an explicit instruction from the owner for this handoff.

## Decisions, intended scope, and unresolved choices

The owner selected self-contained game profiles with no automatic base-content inheritance.
Read docs/34-self-contained-profiles.md before designing content storage/loading. The future
launcher chooses the profile; launcher construction is not a prerequisite. Each profile has its
own account settings, characters, levels, and progress. Profile data format and the storage layout
that implements this isolation remain unresolved; decide them before dependent work.

| Item | Status | What the implementer must do |
|---|---|---|
| Rust owns simulation, movement, pathfinding, combat, and AI execution | Chosen | Build on the existing Rust world |
| Lua is the content scripting language | Chosen | Do not reopen Rhai versus Lua |
| BehaviorTree.CPP defines behavior-tree execution rules | Chosen | Follow ADR 0014; implement in Rust |
| HeroAI supplies combat decision logic to port | Chosen direction | Preserve decisions while mapping them to server state and actions |
| Database stores installed content and durable game state | Existing architecture | Use asura-db; load active simulation state into memory |
| A small content manager arrives with configurable maps | Intended Phase 1 scope | Deliver an editing interface, not only SQL instructions or a CLI |
| JSON import/export for contributors | Earlier recommendation, not owner-selected | Ask before making this a required workflow; do not block independent map work on it |
| Event-driven Lua handlers | Proposed approach | Settle the event contract before Phase 6; Lua selection alone does not settle it |
| Manager UI technology and delivery | Not chosen | Ask the Phase 1 interface question before building the app shell |
| Spawn-selection, party ownership, persistence, and phasing rules | Partly recorded; partly open | Read existing decisions first; ask only about what remains unresolved |

Operational configuration remains with config/services.toml, environment, and the supervisor.
Map definitions and NPC/quest definitions are content. Health, cooldowns, inventory, and earned
experience are changing game state. Wire constants and field layouts belong to the protocol.
Do not turn all hardcoded numbers into operator settings.

A manager, a JSON importer if selected, and Lua actions must use the relevant shared Rust
operations. Do not create separate implementations of content validation, travel, combat, or
progression in each interface.

## Entry check for the implementation agent

1. Inspect git status and identify existing work. Do not overwrite, revert, or include unrelated
   changes in the implementation. Several movement changes were present during this handoff.
2. Check the actual code and current recordings, not only status paragraphs in older documents.
   Refresh file:line references if they moved. Do not assume a listed gap still exists.
3. State the active phase, the exact feature being changed, and its source of truth.
4. Apply AGENTS.md's pre-flight sentence before behavior changes. Content extraction and a
   gameplay-policy change are separate subjects.
5. Resolve only the decision gates needed for that phase. Missing later-phase decisions do not
   prevent independent earlier work.

Source locations inspected while preparing the plan:

| Location | What to verify |
|---|---|
| crates/asura-db/src/game.rs:55 | Default map settings read from map_config |
| crates/asura-db/src/game.rs:69 | map_spawn list reader; no caller under crates/ was found in the review |
| crates/asura-db/migrations/0020_map_spawn_points.sql | Existing spawn-list schema and row-zero seed |
| crates/asura-world/src/map.rs | In-memory map definition and shared path graphs |
| crates/asura-world/src/portals.rs:15 | Portal shape; current source/destination/arrival data |
| crates/asura-world/src/portals.rs:32 | Current reach value 250, documented as unmeasured |
| crates/asura-world/src/portals.rs:39 and :71 | Ordered crossed-door and discovered-door tables |
| crates/asura-world/src/travel.rs:408 | Arrival x/y is written before transfer, using plane 0 |
| crates/asura-world/src/connection.rs:254 | Current selection from map default and saved position |
| crates/asura-world/src/instance.rs:19, :34, :368 | Player slots, instance state, and registry keyed by map id |
| crates/asura-world/src/player.rs:144 | Experience test value 500 rather than maintained progression |
| crates/asura-protocol/src/game/mod.rs:335 | Fixed health/energy values used by existing builders |
| config/services.toml and docs/19-running.md | Existing process management and run path |
| docs/22-world-model.md, "Spawn points, and the outpost rule" | The owner's existing entry, travel, and last-outpost decisions |
| docs/31-heroai-server-feasibility.md | Combat-policy review and algorithmic scaling concerns |
| docs/adr/0014-behavior-tree-execution.md | Pinned sources, execution differences, and migration checks |

The portal radius being unmeasured is not permission to replace it with a guessed better value.
The existence of multiple spawn rows is not permission to activate a new selection rule while
extracting the current configuration.

## Phase 1 - Configurable map travel and the first manager

### Goal and fixed scope

Deliver database-backed map entry and portal-arrival configuration, editable through a small
manager, and consumed by the world without a Rust rebuild.

The initial editing and client acceptance flow covers:
- Ascalon City, map 148.
- Lakeside County, map 146.
- Their known crossings in both directions.

Preserve all other existing map and portal data. Restricting the first screen and client example
to two maps is not permission to delete the remaining catalogue or recorded crossings.

### Questions to ask before dependent work

- Should the first manager be a local browser app or a desktop app? Where should it run relative
  to the server/database? Do not choose a new UI framework or launch model before this answer.
- Is JSON import/export wanted in this first delivery, or should it remain later work? It is not
  a substitute for the editing screen.
- If the current map_config default and map_spawn row zero disagree, which recorded value is
  authoritative? First check their sources and the live behavior; do not silently select a winner.

Do not ask again about Rust, PostgreSQL, Lua, or the need for the small manager. These are settled
at the level needed here. If the assigned environment cannot deliver the selected manager form,
report the concrete limitation rather than replacing it with a different product.

### Work packages, in order

1. Inventory the default spawn fields, spawn-list rows, portal rows, readers, and travel consumers
   for the selected maps. Record which runtime paths consume each value.
2. Preserve the current values and their evidence when migrating compiled portal content into
   database-backed definitions. Keep discovered/unlinked doors distinct from working crossings.
3. Give content rows stable identities. Keep map identity, portal content identity, live instance
   identity, and agent identity distinct. A generated row number is not a measured client agent id.
4. Establish one authoritative read model for the supported defaults and portal arrivals.
   Maintain existing compatibility fields consistently where readers still need them. Updates
   spanning related rows must not leave the default and its list disagreeing after a failed save.
5. Preserve current trigger precedence. CROSSED is searched before PORTALS and there are overlapping
   entries. Do not deduplicate, reorder, or reinterpret them merely because the database has keys.
6. Preserve nullable destinations and arrivals. An unmeasured destination is not map 0. Do not
   require inventing a destination to import an inactive discovered doorway.
7. Load definitions into the world's memory through asura-db and connect the existing travel
   paths to them. Reuse the current transfer cycle. Do not query the database on every movement
   tick just to find a static portal.
8. Build the first manager screen and save operation against that same content model.
9. Document how to start the selected manager, save, restart the world, and verify the result.
   Extend the existing supervisor only as required by the owner-selected delivery form.

Initial editable information:
- The selected maps' current default spawn x, y, and plane.
- Existing known portal links' destination and landing x, y, and plane.

Show map/portal identity and source location so the owner can identify the record. A plane is a
pathing plane, not terrain height. Do not infer height or renumber planes.

Use the existing data representation for additional spawn rows and preserve them. Making
individual extra rows editable, adding/removing records, exposing source trigger coordinates or
radius, and adding random or conditional selection are separate feature choices; ask before
adding them to the first screen.

### Behavior boundaries

- Extract the current behavior first, preserving values and the order/meaning of emitted messages.
- A deliberate coordinate edit is the explicit configuration test, not a new default to commit.
  Restore the original development data after that test unless the owner says to retain the edit.
- Initial changes take effect after the world restarts. Do not add live mutation of active
  instances or hot reload to satisfy this phase.
- Do not change portal detection geometry, collision, movement, travel permissions, capacity
  policy, reconnect behavior, last-outpost rules, or the explorable initialization workaround.
- Normal entry selection, portal arrivals, quest placement, and reconnect/session placement are
  different cases. Read the existing owner decisions in docs/22-world-model.md. Implementing the
  missing cases belongs to separately assigned behavior work; do not smuggle it into extraction.
- If connecting an unused reader would change current placement, document that difference and
  resolve it before wiring the new selection into live behavior.
- The map editor is the content part of the broader control panel in docs/33-control-panel-design.md.
  Server operations and capture/test workflows are now explicitly requested panel capabilities.
  User/account administration, a visual map canvas, generic database browsing, and production
  remote administration are still unselected features, not consequences of that request.

### Validation and completion

Reject malformed coordinates and invalid required references with a useful record/field error.
Allow deliberately incomplete inactive doorway records. Validate planes against available
geometry where that evidence exists; do not pretend an unavailable graph proves validity.
A failed save leaves the previously valid records intact. A successful save must be read back
by both the manager and the world loading path.

Automated checks are limited to the touched loader, database/schema operations, and validation.
Follow AGENTS.md for shared-schema checks. Do not build a new general test framework.

Client completion steps:
1. Load the original data and travel through the known crossings in both directions.
2. Save one deliberate valid portal arrival change in the manager.
3. Restart through the supervisor, cross that portal, and confirm the configured arrival.
4. Test the selected map's default entry through the appropriate default-entry path; a portal
   crossing alone does not prove the default spawn is read.
5. Restore original test data and verify it is loaded again.

Phase 1 is complete only when the screen, save, world consumption, and real-client path all work
without rebuilding Rust for the content edit. Backend-only work is partial. If client testing
cannot be performed in the assigned session, report it as pending and give the owner the exact
steps; do not mark the phase client-confirmed.

## Phase 2 - Independent running instances

### Goal and work

Separate a map definition from each running copy. Replace the assumption that map_id alone
identifies all mutable state. Carry the actual assigned instance through reservation, travel,
connection claim, and reconnect, retaining the measured client protocol.

Reuse existing registry, capacity, transfer, and token mechanisms. A map definition and path
graph can be shared; agent state, timers, encounter state, and dropped items belong to one copy.
Extend placement and leases where needed instead of creating an unrecorded auth/world side channel.

### Ask before implementing unresolved policies

- How is a party identified and which party assignment is authoritative?
- When does a party reuse an instance, get a fresh one, or retain one after disconnect?
- What happens when a party changes membership or leadership?
- What distinguishes a district assignment from a private explorable assignment?

First check existing decisions and captures; do not derive party identity from an account or
session just because that column is available. The intended solo-isolation example does not
authorize resetting unrelated public-district behavior.

### Completion and exclusions

Two solo parties can enter independent copies of the same map. Players assigned to the same
copy share its changing state. A reconnect claims its assigned copy under the chosen policy.
Ending an empty copy releases its transient state without touching the other copy.
Exercise capacity refusal and stale-assignment handling in the affected path.

No new phasing system, cross-world social system, or instance-migration service is included.
Do not mark party sharing complete if only the solo allocation path exists.

## Phase 3 - NPCs owned by the instance

### Goal and work

Add live agents independently of connection-oriented player slots. Extend the manager's content
model with only the definitions, placements, groups, and routes needed for the selected NPC.
Reuse existing pathfinding and movement; do not replace the accepted movement implementation.

The instance owns each agent's identity, position/plane, movement, and lifecycle. Shared
definitions describe appearance, faction, and assigned behavior; mutable state is separate per
agent and per instance. Introduce actor-relative nearby queries for later combat searches.

Use one owner updating an instance at a time and bounded workers across instances, consistent
with docs/02-architecture.md. Network/database waits must not block CPU simulation work.
Thread count is not the number of agents or instances.

### Ask before unresolved content or behavior

Select the first real NPC, its measured appearance and entry messages, placement, and patrol.
Ask if those are not specified or evidenced. Do not invent an NPC model, faction behavior,
route, roam radius, respawn rule, or server/client-visible flag to make a demonstration pass.

### Completion and exclusions

The selected NPC is content-defined, appears correctly to a real client, and walks the specified
route. All players in one instance see the same NPC state; another instance has independent
NPC state. Joining/leaving, NPC removal, and teardown do not leave ghost or shared agents.

This phase establishes appearance and movement. Combat, quest availability, loot generation,
and generic population/respawn policies are later work. A stationary model alone is not a
completed patrol.

## Phase 4 - Authoritative combat

### Goal and work

Build a small real combat loop before connecting general HeroAI decisions to it. Maintain
health, resources, effects, targets, attacks/casts, completion times, and death in Rust.
Player requests, NPC decisions, and later Lua requests use the same relevant world operations.

Choose the first evidenced attack and small set of skills. Implement their legality checks,
resource use, timing, result, state updates, and outgoing messages. Build load and update values
from the same state. Replace the affected stand-ins as their owning systems arrive.

### Ask before unresolved rules

Ask which initial encounter/skills to reproduce if not already assigned. Obtain missing rules
and packet evidence through the repository's source order. Do not invent damage formulas,
cooldowns, regen, aggro/leash ranges, resurrection, rewards, or respawn timing.
Experience 500 and fixed health/energy values are evidence of existing stand-ins, not balance
defaults to propagate into the final simulation.

### Completion and exclusions

A player and NPC can perform the selected combat actions; resources and health change once at
the correct points; the real client sees consistent results; death changes authoritative state.
Verify the affected interrupted/invalid-action path where the selected action requires it.

Do not call the game's full combat/skill system implemented after this example. Do not add
unrequested balance controls or configure packet constants through the manager.

## Phase 5 - HeroAI policy on the Rust combat engine

### Goal and work

Port the decision logic reviewed in docs/31-heroai-server-feasibility.md against explicit server
actor state and actions. Follow the pinned execution reference and mapping procedure in ADR 0014.
The intended reuse is skill selection, targeting, conditions, and coordination policy, not the
client runtime.

Start with the generic path and selected skillbars whose actions are implemented. Identify
specialized build dispatch that the real selection uses; report unsupported branches explicitly.
Share immutable definitions and retain separate actor execution state. Avoid repeated skillbar
preparation when inputs have not changed and measure repeated target scans before claiming scale.

### Required compatibility checks

For each affected branch, record:
- Original node and callback behavior, including caller checks outside the BT.
- Exact BehaviorTree.CPP variant used.
- Changes in immediate completion versus the Python extra tick.
- Whether earlier priorities/guards are rechecked while another child runs.
- Completed parallel children, repeat failures, and reentry behavior where applicable.
- What halting a branch does to a requested move/cast and who owns that action afterward.

Ask about an adaptation that changes gameplay intent or observable reaction timing. Do not
silently preserve every Python quirk or silently replace the policy with a smarter one.
Source implementation details are not permission to give all enemy types the same tactics.

### Completion and exclusions

The selected NPCs choose and perform valid skills/targets through the Rust combat engine, with
the affected decision and node-lifecycle checks recorded. Client-visible behavior and packets
are checked on the selected encounter. Unsupported skills and builds are listed, not replaced
by success stubs. Client shared-memory/cache replacement is not listed as an unsolved obstacle.

A Rust port alone does not establish 20,000-agent capacity. The owner has not selected a global
AI frequency or authorized changing reaction speed to reach that figure.

## Phase 6 - Lua through a complete quest

### Goal and work

Embed the chosen Lua runtime and expose the existing Rust actions needed by one selected quest.
Decide the scripting contract before implementing handlers: event source/order, scope, when
requested actions apply, and behavior after a handler fails. Use simulation events rather than
reimplementing combat in Lua.

The first path is quest giver -> acceptance -> selected destination or enemy objective ->
completion -> persistence. Add escort/follow sequencing after that path works.
Ordinary objectives and their relationships belong in the selected content representation;
Lua coordinates the exceptional sequence using world operations.

### Ask before unresolved scripting and quest policies

- Which real quest and objective are the first acceptance case?
- Are handlers the selected authoring model, or are suspended quest flows also required now?
- Which progress is character, party, or instance state, and who receives objective credit?
- Which party progress chooses a population/phase when members differ?
- What are the accepted error, execution-limit, reload, and permitted-access policies?

Do not treat Lua selection as approval of any particular API, JSON schema, event frequency,
coroutine model, or hot-reload behavior. If a dependency requires raising the repository's Rust
toolchain requirement, ask before doing that.

### Completion and exclusions

A real client can interact, accept, satisfy the objective, complete the quest, and retain its
required progress across logout. Repeating an event does not duplicate completion or rewards.
If this quest includes rewards, use the selected measured reward rules and existing state
operations. A fetch quest also needs the missing pickup/objective path, not just inventory rows.

No direct Lua implementation of packets or database writes replaces the Rust operations.
A full quest editor, branching campaign, general phasing system, and live script migration are
outside the first quest unless explicitly added by the owner.

## Phase 7 - Measured capacity

### Goal and work

Measure the populated simulation rather than extrapolating from empty maps or a language
microbenchmark. Test one 200-agent instance first, then concurrent instances toward 100.
Include dense target selection, varied skillbars, healing/effects, interrupts, movement,
and relevant quest events. Distinguish idle from all-active combat.

Record hardware/build, content and workload, decision/update rates, CPU, memory, update latency,
missed deadlines, path searches, and outgoing traffic. Record actual counts, not only a target.
Keep client-visible timing and the simulation/network rates distinct.

### Ask before unresolved acceptance targets

Ask which hardware and latency/headroom targets define success. No previous illustrative
10-decisions-per-second example is a chosen rate. Do not claim one machine must support the
entire workload, nor silently lower fidelity or freeze relevant agents to achieve it.

### Completion and exclusions

Publish reproducible measured capacity per world process/machine and identify the limiting
work. Validate representative gameplay with the real client. Report if 100 instances could not
be run and why; do not turn a projection into a measured result.

Do not build a general benchmark platform during Phase 1. Any load tooling here must serve
this workload and follow the test-scope rules for this milestone.

## Progress and reporting

All phases remain planned in this handoff. Mark a phase implemented, automated-checked, and
client-confirmed separately. Existing features that a phase reuses retain their own evidence;
the existence of a type, table, packet builder, or manager button does not prove the full path.

| Phase | Next visible result | Status |
|---|---|---|
| 1 | Edit the selected maps' spawns/portal arrivals through the manager | Planned |
| 2 | Independent solo copies and the selected party-sharing policy | Planned |
| 3 | A configured NPC walking its route | Planned |
| 4 | A player and NPC resolving the selected combat actions | Planned |
| 5 | NPC skill and target decisions driven by ported HeroAI policy | Planned |
| 6 | One quest playable through completion and saved progress | Planned |
| 7 | Measured capacity with populated instances | Planned |

At each handoff, report the exact completed work, changed files, one-line check result,
real-client result or pending client steps, and the next actual unresolved question.
Record decisions in the existing decision ledger; replace text that a decision supersedes.

Start tests with tools/test-touched.ps1. Run only the commands justified by the assigned change.
If its status scan includes pre-existing work, identify that before choosing test scope.
Follow AGENTS.md for schema/shared-machinery checks and whole-suite justification.
Run the ASCII check for written files. No unrelated cleanup or source/project dependency enters
a phase merely because the implementer is already editing nearby code.

## Starter instruction for the next implementation session

Read AGENTS.md and docs/HANDOFF.md, then docs/32-world-simulation-plan.md.
For control-panel work, also read docs/33-control-panel-design.md and implement only the panel
increment assigned by the owner; resolve its deployment/interface questions before dependent work.
For simulator Phase 1, implement configurable default spawns and existing portal arrivals for
the initial map 148/146 editing flow, with its panel editing interface. Preserve other content
and the current travel behavior during extraction. Check the Phase 1 decision gates.
Ask about unresolved requirements instead of selecting features; complete independent
authorized work while waiting. Report the real-client acceptance steps and their actual status.

