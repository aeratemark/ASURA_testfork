# Skill mechanics and scripted behavior

Research and owner clarification recorded 2026-11-02. This is documentation for later work;
no combat runtime, scripting API, or skill implementation is introduced by this review.

## Owner's distinction: scripts declare behavior; the server executes mechanics

This is a simulation-wide distinction: NPC decisions, movement behavior, combat, skills, quests,
spawns, encounters, loot, dialogue, interactions, and map behavior belong in external scripts.
Mechanics are the supported server capabilities that execute that behavior. Scripts define
rules, conditions, calculations, sequences, and reactions; Rust executes the requested operations
against authoritative state. Skills below are examples of this general boundary, not its scope.
The full responsibility table is in docs/30-world-content-and-scripting.md.

The owner's example is flight: if the server offers no flight mechanic, scripts cannot fly.
A declaration cannot supply a missing server capability. This distinction also applies to
skills: declaring a projectile, damage, or an enchantment requires those server mechanics.

| External skill behavior declares | Server mechanic executes |
|---|---|
| Which targets qualify and what conditions change the result | Target queries and validation against authoritative state |
| Resource requirements, activation, recharge, and duration values | Resource accounting, cast scheduling, interruption, and timers |
| Launch a projectile; on impact, damage the target | Projectile movement, hit resolution, damage calculation, and health changes |
| Apply regeneration while an enchantment lasts | Effect ownership, regeneration accounting, expiration, and removal |
| Limit eligible incoming damage while an effect lasts | Supported damage adjustment operations at the appropriate resolution stage |
| React to an attack by damaging the attacker | Event delivery and execution of the requested damage operation |
| Which skill an NPC chooses and who it targets | Execution of the chosen action through the same world operations |

These are skill-specific examples of the general boundary, not selected API names or a file
format. Rust behavior-tree execution remains a server mechanic; game-specific decisions belong
in external behavior scripts. How scripts connect to that runtime, and the exact events and
operations exposed to Lua, remain open. Scripts run through the server; they do not independently change client state or send
invented packets. See [30-world-content-and-scripting.md](30-world-content-and-scripting.md).

## Rurik source review

Checkout reviewed: C:/Users/Apo/Documents/GitHub/Rurik at commit
0710df3b5cb068362c61120820753e59a36dd149.
Paths and line numbers below refer to that revision, not ASURA. Rurik's source is the only
witness for these implementation findings. No Rurik server, tests, or client session was run
for this review; its cited recordings were not independently replayed into ASURA evidence.
Rurik remains a research source, never a runtime dependency (ADR 0011).

| Capability | Implementation found | Source at reviewed revision |
|---|---|---|
| Player requests | Handles ordinary skills and attack skills with activation gates | toolkit/authsrv/authsrv.py:27210 |
| Cast execution | Deferred resource payment and animation at actual queued-cast start | toolkit/authsrv/authsrv.py:27939 |
| Cast timing | Pending records advanced through completion, aftercast, and recharge | toolkit/authsrv/authsrv.py:28046 |
| Skill data | Timing comes from content; magnitude meaning is read separately from its value | toolkit/authsrv/authsrv.py:24725, 5447; toolkit/authsrv/skillread.py |
| Lasting effects | An effect episode opens and closes on its bearer | toolkit/authsrv/effects.py:1 |
| Projectiles | Player spell launch and later impact have separate handling | toolkit/authsrv/authsrv.py:17020 |
| Reactions | Active hexes marked as attack triggers damage the attacker | toolkit/authsrv/authsrv.py:29252 |
| NPC execution | Separate hostile attacks, ally casts, and ally attacks | toolkit/authsrv/authsrv.py:32526, 33301, 37368 |

Important limits and divergences:

- The ordinary recharging cast sequence is E4, E5, E3, E6, but it is not universal.
  completion_sends_e5 omits E5 for zero recharge under the current setting, and cast_tick
  documents that zero-recharge cycles close at E3 without E6
  (toolkit/authsrv/authsrv.py:28034-28062).
- begin_cast explicitly states that its queue stacks requests where retail replaces the queue.
  Its extra begin-time resource check and remaining busy window are documented consequences,
  not a retail queue specification (toolkit/authsrv/authsrv.py:27956-27966).
- An unmodeled damage skill returns None, not zero; missing timing content instead falls back
  to zero with a log message. These are different reference behaviors, not ASURA defaults
  (toolkit/authsrv/authsrv.py:5447, 24725).
- The old land_skill docstring describes only the player as target, but later implementation
  notes explicitly record target parameterization for party casts. Read the current call path
  rather than treating that older description as the full capability
  (toolkit/authsrv/authsrv.py:40451, 33314-33320).

## NPC behavior found in Rurik

| Decision | Implementation found | Source |
|---|---|---|
| Choose a skill | Round robin over ready slots, skipping disabled skills | toolkit/authsrv/authsrv.py:40388 |
| Choose a heal target | Lowest health fraction below HERO_HEAL_AT, set to 0.9 | toolkit/authsrv/authsrv.py:33172, 15031 |
| Choose an eligible target | Enemy, ally, self, and other-ally paths; offensive range checks | toolkit/authsrv/authsrv.py:33400 |
| Hold an unnecessary effect | Checks whether the primary wearer already carries the effect | toolkit/authsrv/authsrv.py:32429, 33449 |
| Remove an affliction | Removal targeting checks for the applicable affliction | toolkit/authsrv/authsrv.py:33426 |
| Pursue or return | NPC following and return-to-spawn handling | toolkit/authsrv/authsrv.py:37458, 38891 |
| Fight from casting range | Hostile caster classification and range handling | toolkit/authsrv/authsrv.py:33832 |
| Escape damaging areas | Scatter state and movement handling | toolkit/authsrv/authsrv.py:39414 |

pick_skill explicitly labels its round robin a testing function, not measured ArenaNet AI
(toolkit/authsrv/authsrv.py:40388). Healing target selection also declares itself authored
policy (33175). These policies must not become claims of live-game parity through a port.
The primary-wearer effect check also documents incomplete coverage for area effects (32444).
The monster AI research is studies/monsterai/FINDINGS.md; it distinguishes observable outcomes
from the unavailable original server decision function. That study is research, not proof of
any unmeasured selection, aggro, or leash rule.

## Skills that illustrate the distinction

The following wiki values were researched for explanation at attribute rank 12, without other
modifiers. Wiki descriptions are not ASURA captures and do not establish packet layouts,
processing order, rounding, or timing details. Rurik implementation evidence is listed separately.

| Skill | Published behavior at rank 12 | Script declaration and required mechanics |
|---|---|---|
| Flare | 5 energy, 1 s activation, no recharge; 56 base fire damage on projectile hit; Overcast adds adjacent foes | Declare projectile and conditional impact behavior; server executes flight, hit resolution, targeting, and damage |
| Healing Breeze | 10 energy, 1 s activation, 5 s recharge; +8 regeneration for 15 s | Declare a timed enchantment contributing regeneration; server maintains and removes its contribution |
| Protective Spirit | 10 energy, 0.25 s activation, 5 s recharge; 19 s protection limiting eligible damage to 10% maximum health | Declare the protection behavior; server must expose the necessary damage-resolution operation |
| Empathy, PvE | 10 energy, 2 s activation, 10 s recharge; 13 s hex, 46 reactive damage, attacks deal 12 less damage | Declare the attack reaction and outgoing damage adjustment; server supplies events and damage operations |

Wiki sources consulted in the discussion:

- https://wiki.guildwars.com/wiki/Flare
- https://wiki.guildwars.com/wiki/Healing_Breeze
- https://wiki.guildwars.com/wiki/Protective_Spirit
- https://wiki.guildwars.com/wiki/Empathy
- https://wiki.guildwars.com/wiki/Health_regeneration
- https://wiki.guildwars.com/wiki/Aftercast_delay
- https://wiki.guildwars.com/wiki/Event_stack

The research identified why shared mechanics need distinct operations: regeneration is not
healing, degeneration is not damage, and Protective Spirit does not reduce life stealing or
health loss. Empathy reacts even to missed or blocked attacks; successful-hit events alone do
not describe it. Interactions and processing order still need live verification.

Rurik examples to revisit:

- Flare's content identifies fire damage (content/world.toml:144). Its projectile speed is
  reconstructed from Fireball sharing the projectile, not a recorded live Flare cast (2925).
- Healing Breeze's content identifies regeneration and cites a live recording combining its
  contribution with degeneration (content/world.toml:3335). This is Rurik's evidence claim.
- Empathy-style attack reactions use each active hex's stored caster and attribute rank to
  request armor-ignoring damage (toolkit/authsrv/authsrv.py:29252). This does not establish
  complete reproduction of every part of Empathy, including its damage reduction.
- Protective Spirit was a conceptual example in this discussion; this review did not establish
  a complete Rurik implementation of it.

## Deferred work

Implementation is deferred at the owner's request. No skill set, encounter, API, script schema,
event ordering, or missing mechanic is selected for implementation here. Before a later port,
separate the reference's measured execution from its authored policies and divergences, define
the script/server contract, and obtain ASURA's own evidence through the repository source order.
