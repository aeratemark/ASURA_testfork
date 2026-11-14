# Pre-Searing behaviors

Scope: the six explorable maps 145-148 and 160-166 of the pre-Searing pack. The Plague Worm cycle is
implemented (`docs/59`). This document records the other oddities the owner named, what the internet
and the owner establish, and what each script still needs before it can be written. It is research,
not an implementation claim.

The owner's observation outranks every fan page below. A fan page gives a shape or a count; it is
not a packet record. A capture or a Py4GW Stealth dump of the live client is still the measurement
that turns a shape into a script.

## The four the owner named

| NPC or group | What is specific | Evidence | What a script needs |
|---|---|---|---|
| Plague Worm, Lakeside (map 146, `mob-1442-type`) | emerge, stand out, submerge, remove, re-appear at the same id and coordinates; never walks; stays up in a fight; spits from range; out of a fight, 5-10 s up then 5-15 s under, drawn per cycle | Rurik `studies/tape/FINDINGS.md` T3, `studies/cmsg/FINDINGS.md` T16; owner's live client 2026-11-14; implemented | built: `worm_lakeside` in `npcs/behaviors.lua`, `stationary` and the captured projectile in `definitions.json` |
| Black Bear, Regent Valley and Wizard's Folly (`mob-1354-type`, `mob-1352-type`) | `Breaking Charm` interrupts `Charm Animal`; `Brutal Mauling` is the slow attack that opens the taming trick; pre-Searing bears wander widely and aggro on their own until they gain a level | [Black Bear](https://wiki.guildwars.com/wiki/Black_Bear), [Guide to taming a bear in Pre-Searing](https://wiki.guildwars.com/wiki/Guide_to_taming_a_bear_in_Pre-Searing); owner's observation | two skills and their cast/interrupt timing; a charm attempt that can be interrupted; the post-tame anomaly |
| Bear Hunters, Regent Valley (southwest of Fort Ranik) | five drinking hunters: Benton, Bruse, Dannel, Neihl, Senssa. They bark, play "Blood, Sweat, and Tears", one wanders off and returns, and the group runs the repeatable bear-kill minigame for Hunter's Ale | owner's observation; [Bruse](https://wiki.guildwars.com/wiki/Bruse), [Benton](https://wiki.guildwars.com/wiki/Benton), [The bear hunters](https://wiki.guildwars.com/wiki/The_bear_hunters) | dialogue lines and choices, a local timer, random idle barks, a short wander, quest offer/reward/cooldown. The five are absent from the current pack; a capture must supply their file ids, models and coordinates |
| Deadly Crypt Spider, The Catacombs (map 145) | about twelve drop down from the ceiling onto the descending path between the two skeleton bands, near the Flame Geyser chambers. Before the trigger they are not bodies | owner's observation; [Deadly Crypt Spider](https://wiki.guildwars.com/wiki/Deadly_Crypt_Spider); [area guide](https://presearing.boards.net/thread/2499/area-guide-catacombs) | a trigger line or proximity check, a `spawn` at a ceiling point, and a fall/drop transition. The pack's map 145 currently seeds every body at instance creation |

The fan guide's sentence is the only count: "Approximately 12 of them will drop down onto that
path, just between the two areas where bands of skeletons roam." It does not give the trigger
distance, the ceiling height, or the drop duration. Those are not in this tree.

## Other Catacombs behavior

The Catacombs is map 145 in the pack, not map 148 (148 is Ascalon City). The area guide and the wiki
describe the rest of it:

- Gargoyles and Shatter Gargoyles sleep in the far north and north-west. A player can walk past the
  sleepers; a Shatter Gargoyle wakes and attacks.
- Crypt Fiends and a Tomb Nightmare stand in the inner Ritual Chamber.
- Restless Corpses and Raging Cadavers fill the large regions from the Ashford Abbey entrance to the
  west and north.
- Flame Geysers are interactive objects, not NPCs.

None of these has a measured wake radius or spawn trigger here. The `mob-1518-type`,
`mob-1417-type`, `mob-1407-type`, `mob-1405-type`, `mob-1494-type` and `mob-1424-type` rows in map
145 are the candidates; the pack does not carry their names, so the match between a row and a wiki
creature is not established. A Py4GW dump of map 145 names each body.

## What the pack has and has not

- Map 146: 18 `mob-1442-type` Plague Worms, now `worm_lakeside`, and the Worm Queen
  (`mob-1445-type`, the quest spawn of "The Worm Problem") with the one-shot `worm_queen` emerge;
  5 `mob-1444-plague-worm` in map 162 (Regent Valley) stay unscripted. Rurik measured definition
  1442 in Lakeside, not 1444.
- Map 161: five `mob-1352-type` bears. Map 162: seven `mob-1354-type` bears. Both have empty skill
  lists in `definitions.json`; the bear script cannot run until the two skills exist.
- Map 162: the Bear Hunters are not in `npcs.json`. The named rows are Mary Malone and an Academy
  Monk; the rest are bandits, Grawl, bears and worms. The hunters are a content addition, not a
  behavior-only change.
- The ale itself is a held item or a reward, not a body. The current pack has no held mug on any
  hunter.

## The operations the next scripts need

Each row is a Rust mechanic, not something a Lua table can invent (`docs/30`, `docs/35`):

| operation | why | first user |
|---|---|---|
| `trigger_line` / `trigger_radius` | a body or group becomes active when a player crosses a line or enters a radius | spiders, sleeping gargoyles |
| `spawn` / `despawn` at a point | create a body that did not exist at instance seed, and remove it later | spiders, boss pop-ins |
| `say` / `dialog` / `choice` | encoded chat lines and the client's dialog choices | Bear Hunters, quest NPCs |
| `skill` / `interrupt` | `Breaking Charm` cancels a `Charm Animal` cast; `Brutal Mauling` is a slow attack | Black Bear |
| `wander` / `face` / `pause` | the hunter who walks off and returns; the bear's wide pre-Searing wander | Bear Hunters, Black Bear |
| `timer` / `random` | the three-minute bear hunt and the idle bark order | Bear Hunters |

The first three are the next shared prerequisite after `Burrow`: a body that does not exist until a
trigger, and a body that can speak. The bear's two skills are combat work, not NPC scripting; they
need the skill engine's interrupt and attack-skill mechanics first.

## Sources

- Owner's observations, 2026-11-14: the ale drinkers near Fort Ranik, the bear's tame-cancelling
  skill and the trick that still works, the Catacombs mobs, and the spiders that appear from a line.
- Rurik: `studies/tape/FINDINGS.md` T3 and `studies/cmsg/FINDINGS.md` T16 for the worm.
- Guild Wars Wiki: the creature and guide pages linked above.
- presearing.com and presearing.boards.net: the Catacombs area guide and the bear-taming thread.
  These are fan documentation, not a client capture.
