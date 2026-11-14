# 57 - Damage numbers: why a hit showed 0, and why the swing ended after it

**Status, 2026-11-13:** three fixes from one client report. The damage number was 0 on every hit, and a
hammer's hit appeared a moment after the swing. The XP bar was also still: its own section is
`docs/54-experience.md`.

## 1. The damage number is a fraction of the target's maximum health

The damage word (`0x00A3` kind 16, or kind 55 for armour-ignoring) carries a **signed fraction of the
victim's maximum health**, not a point count: Rurik measured 1,668 such words, and every one whose
taker's maximum was known is a whole number of points over that maximum
(`docs/36-game-rules-ledger.md` E181-E182; `Rurik/toolkit/authsrv/authsrv.py:1705-1774`). The client
converts it back as `round(fraction * max health)`.

This server sent the fraction, correctly, but never sent the **maximum** for a creature. Property 42
(`0x009F`, maximum health) was sent for a player at load and for no NPC; a client multiplying the hit's
fraction by zero shows zero. The live load capture does not send it either (`docs/49` 2.5: "no NPC
burst sends it"), which is why the gap was invisible until a hit landed.

**Fix:** an NPC with a combat block gets property 42 (its `NpcCombat::health`) and property 34 (the full
health fraction) at the end of its entry burst (`crates/asura-world/src/npc/burst.rs`), which is what
preforged's new-observer path sends (`gw_world_simulation.h:15737`, `health_packets_2022`). An NPC
without a combat block gets neither, because it cannot be hit.

## 2. The hit landed at the end of the interval, not at the windup

An ordinary attack is `Engine::attack`, not a bar skill. It opened with kind 4 (`0x00A0`), then sent
its damage when the **whole attack period** had elapsed. The client animates the swing from the
`0x0035` pair (interval and modifier) and its contact is at the windup, not at the end: retail's law is
`interval / 2 - 0.1 s` (Rurik, `toolkit/authsrv/authsrv.py:20165-20172`, derived over
1.33/1.75/2.0/2.475/3.0 s weapons). A 1.75 s hammer contacts at 0.775 s; this server sent the word at
1.75 s, after the animation had finished.

**Fix:** `swing_windup(interval)` is `interval / 2 - 0.1 s` (clamped to 1 ms). A melee swing lands at
`start + windup`; a ranged one launches its missile at `start + windup` and lands it `windup + flight`
later. The **next** swing still opens one full interval after the previous one started
(`Actor::swing_restart`), so the cadence is unchanged.

Property 1, the melee finish, **opens** the hit batch before the damage word: retail does on 1,376 of
1,376 live hits (Rurik, `authsrv.py:20206-20218`). The client latches the attacker on property 1 and
splices the damage into that agent's pending animation; with the finish after the word the number waits
for the next animation - the owner's delayed hit, and on the worst reads a double one. GWCA calls it
`melee_attack_finished`; preforged sends it as `finish_2022`. The local order is now finish, skill,
damage, health fraction.

The tests: `an_ordinary_swing_lands_at_its_windup_not_at_the_end_of_its_interval` (a 1.33 s axe does no
damage at 0.5 s and does at 0.6 s), `the_windup_law_is_the_measured_one` (1.33 -> 565 ms,
1.75 -> 775, 2.475 -> 1137), and the ordinary-attack test asserts the finish precedes the damage.
Attack skills keep their own kind 46 finish; the two are not the same message.

## 3. What is still not built

- Creature health is sent as the full fraction at entry. A creature that regenerates or is healed
  publishes its fraction through the fight (`0x00A2` 34), which already existed.
- Loot is still not built (`docs/52`, "Not built"): a kill drops nothing. This document is about the
  number above the body, not what the body leaves.
