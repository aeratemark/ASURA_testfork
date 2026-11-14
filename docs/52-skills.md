# Skills: the bar, attributes, casting and the pre-Searing skill effects

Ported 2026-11-12 from preforged (`Other_Repos_Research/preforged-main`), on the owner's instruction
"implement Skills from the preforged repo", scope chosen as everything including effects, with
preforged's skill table as the catalogue source. **Nothing here is measured against a live server
yet.** No capture under `recordings/` contains a skill being used; every message below is preforged's
reading of the 2022 client, matched to this build's packet table by layout. A capture of a live cast
decides each row; until then they are `Inferred` in `crates/asura-protocol/src/packets.rs`.

## What is built

| part | where | what it does |
|---|---|---|
| skill pack | `config/World/Vanilla/skills.json` | the 44 skills obtainable in pre-Searing: their numbers from preforged's 2022 table, and an `effects` declaration per skill |
| combat values | `config/World/Vanilla/combat.json` | ranges, player health/energy, regeneration, the damage formula's constants, weapons, armor shares, condition ids |
| attribute rules | `config/World/Vanilla/attributes.json` | each attribute's profession, the rank costs, the points per level |
| decisions | `crates/asura-rules/src/skills.rs` | whether a bar may stand (learned, professions, one elite, three PvE-only, one copy), the four bar edits, attribute edits within professions, cap, points and the refund rule |
| engine | `crates/asura-world/src/skill/engine.rs` | casts, payment, recharge, adrenaline, damage against armor, healing, conditions, lasting and maintained effects and their reactions, fields, chains, knockdown, interrupt, death and resurrection |
| loader | `crates/asura-world/src/skill/catalog.rs` | reads the three files; any unknown word refuses the world's start, naming the skill |
| wire | `crates/asura-world/src/skill/wire.rs`, `crates/asura-protocol/src/game/skill.rs` | engine events to messages, and who hears each |
| character | `crates/asura-world/src/skill/character.rs` | the learned set, bar and ranks, their row, the load block's messages, the request handlers, `/skill` |
| rows | `crates/asura-db/migrations/0034_character_skills.sql`, `crates/asura-db/src/skills.rs` | one row per character: learned ids, bar, ranks |

The instance owns one fight (`Instance::combat`): every player, and every NPC whose type has a combat
block. It is ticked by the instance clock (`Instance::step`), after the NPC table.

## Scripts declare, the server executes

`skills.json` says what a skill does; the engine has the mechanics (`docs/35`). The operations:
`attack`, `damage`, `heal`, `life_steal`, `condition`, `knockdown`, `interrupt`, `effect` (lasting or
`maintained`), `field` (pulses in an area), `chain` (jumps foe to foe), `drain_energy`, `remove_hex`,
`resurrect`, `lose_adrenaline`, `unsupported`. An effect's `mods` are what it changes while it lasts:
regeneration, attack speed, damage taken, flat reduction, Reversal's heal-instead, Retribution's
attacker damage, Backfire/Empathy/Soul Barbs reactions, Blood Renewal's end heal, Aura of
Restoration's spell heal, a glyph's discount and charges, movement speed, arrow damage and explosions.

Values are the skill's own ranges at the caster's rank: `"scale"`, `"bonus"`, `"duration"`, or
`{"low": a, "high": b}`, each `round(low + (high - low) * rank / 15)`.

Three skills are declared `unsupported` and are refused when used: Charm Animal and Comfort Animal
(no pets), Animate Bone Horror (no corpses or minions).

## The wire

Preforged numbers messages for build 37130, this table for 39112. Matched by field layout:

| use | preforged | here | layout check |
|---|---|---|---|
| use a skill / an attack skill | CMSG 68 / 37 | `0x8046` / `0x8027` | skill, copy, target, call byte |
| bar set / load / swap / move | CMSG 90-93 | `0x805C`-`0x805F` | agent + 3, counted list, 4, 3 dwords |
| attribute decrease / increase / load | CMSG 12-14 | `0x800E`-`0x8010` | agent, token, attribute; agent, ids, ranks |
| attack timing | 53 | `0x0035` | agent, two floats |
| attribute points / free / total / snapshot | 55-58 | `0x0037`-`0x003A` | unchanged numbers |
| maintained effect removed / applied | 63 / 64 | `0x0040` / `0x0041` | agent, id; bearer, caster, skill, rank, id |
| effect applied / removed | 65 / 67 | `0x0042` / `0x0044` | agent, skill, rank, id, float seconds; agent, id |
| integer / two-agent integer property | 158 / 159 | `0x009F` / `0x00A0` | kind, agent(s), value |
| float / two-agent float property | 161 / 162 | `0x00A2` / `0x00A3` | kind, agent(s), float |
| adrenaline / spent | 208 / 209 | `0x00D1` / `0x00D2` | agent, skill, copy, units |
| slot, bar, learned set, new skill, copy count | 216-220 | `0x00D9`-`0x00DD` | |
| slot released, activated, recharging, recharged, spent | 226-230 | `0x00E3`-`0x00E7` | agent, skill, copy (+ seconds) |
| effect word | 240 | `0x00F1` | agent, word |

A server message is preforged's number plus one from `0x003F` on and the same number below it; a
client message is preforged's plus two. The property kinds (`asura_protocol::game::skill::PROPERTY_*`)
are values inside a message and are taken unchanged; 41, 42 and 43 already matched this world's load.

A cast, in preforged's order (`gw_skill_casting.h`): activate (`0x00E4`), then the cast start (`0x00A0`
kind 60, or 50 for an attack skill after its `0x0035` timing), or kind 48 for an instant skill. At
completion: the finish (kind 58, or 46 for a melee attack, none for a bow or an instant skill), the
caster and target bodies (kinds 21 and 20), what the skill did, the slot release (`0x00E3`), then the
recharge (`0x00E5`); `0x00E6` when it ends. A refusal is the release alone. Slot messages, adrenaline
and energy go to the caster only; everything a client draws goes to the whole instance.

A player's death is the live pair this world already sends for `/resign` (`0x002D`, `0x0026 {agent,
4}`); a creature's is the effect word with the dead bit.

## The load block

With the skill pack present, the players block now carries the character's own numbers where it
carried the reference's constants: attribute points earned and free (was 50/50), the bar and the learned
set (were empty), the attribute snapshot for the character's professions (was 42 zeros), and maximum
energy, maximum health for its level and the energy regeneration rate (were 25, 400 and 0.0). A level 1
character now shows 100 health. The creation path is unchanged.

## Testing

No quest or trainer teaches skills yet, so `/skill` does (an ASURA test command, in `chat.rs`):
`/skill learn all` learns every pre-Searing skill of the character's professions, `/skill learn <id>`
one, `/skill forget <id>`, `/skill list`. Then the bar is filled in the Skills window in a town.

Damage needs a foe. An NPC type becomes one with a combat block in its definition:

```json
"combat": { "hostile": true, "health": 200, "armor": 20, "energy": 10 }
```

Without it a body stays out of every fight, which is every townsperson the captures hold.

Run (per `AGENTS.md` #3): `cargo test -p asura-rules skills`, `cargo test -p asura-protocol skill`,
`cargo test -p asura-world skill`, `cargo test -p asura-world dispatch`, `cargo test -p asura-db`
(migration count), then the client: learn, equip, cast Orison of Healing on yourself in Ascalon City.

What was checked before delivery, without a cargo registry: the engine, the loader against the
committed pack, the rules and the protocol builders were compiled and their tests run (std plus the real
`asura-protocol` and `asura-rules`); the world's skill modules were type-checked against those two
crates with stand-ins for the database and the socket. The edits to `instance.rs`, `player.rs`,
`connection.rs`, `dispatch.rs`, `chat.rs`, `main.rs`, `npc/` and `asura-db` were not compiled here.

## Mobs

The pre-Searing explorables (145, 146, 147, 160, 161, 162) hold the creatures the live server sent
there, written by two tools from preforged's official-client captures:

- `tools/mobs-from-capture.py` surveys captures (files, folders or a `.zip` read in place) and reports
  per NPC type its definition, health, level, attack interval, skills, hits and deaths, and per map
  visit every body it created.
- `tools/mobs-to-pack.py survey.json config/World/Vanilla` writes one definition per hostile type
  (`mob-<definition index>-<label>`, with a combat block) into `npcs/definitions.json` and the bodies of
  the fullest real visit into each map's `npcs.json` (`mob-<map>-<n>`). The townsfolk-team stand-ins
  preforged had placed for those creatures (a friendly River Skale Tad) are removed; the people are not.

What is recorded and what is not:

| Value | Source |
|---|---|
| Definition, model file, name string, level, profession, team, speed, position, direction | captured |
| Maximum health | captured (`0x009F` 42) where seen; else 80 + 20 x level |
| Attack interval, skills used | captured (`0x0035`, `0x00A0` 50/60, `0x009F` 48) |
| Armor | the wiki's creature formula: 3 x level, +20 warrior, +10 ranger |
| Attribute rank | assumed: 0.6 x level |
| Damage range | fitted from captured plain hits against an assumed victim armor of 25; else 2 + level to 4 + 2 x level |
| Ranged or melee | **captured**: a type is ranged only where the capture recorded a `0x00A4` launch for it; a type that never launched fights in melee. A non-warrior profession is not evidence of range (a wolf is not an archer). The 12 types the capture never saw launch were `ranged: true` with no model until 2026-11-13, and attacked invisibly from range; the River Skale Tad is the one exception (below). The Plague Worms and the Worm Queen carry their captured projectile (143), and the owner's live client shows them spitting from range (2026-11-14), so all three are `ranged: true` and `stationary: true` (they never walk); GWW "Plague Worm" says they "actually fire normal arrows", so their attack is piercing |
| Starts a fight | `NpcCombat::aggression`: `always` (the default) or `when_provoked`. The River Skale Tad (`mob-1433-type`, file 82023, scale 90) is `when_provoked`: it stands aside until a player damages it, then fights that player and forgets the provocation when it walks home (owner's report, 2026-11-13). The Plague Worms (`mob-1442-type`, `mob-1444-plague-worm`, `mob-1445-type` the Worm Queen) are also `when_provoked`: GWW "Plague Worm" says they "will by default not attack", and the owner's live client on 2026-11-14 confirms the ones not hit stay burrowed. The prompt was "mobs that only start attacking after provoked versus mobs that are always hostile" |
| The tad's spit | `mob-1433-type` is ranged with model 1, the adult River Skale's captured spit (same file 82023). The survey never saw a launch because the tad is neutral until hit; the owner's live client shows its blue projectile, so the pack copies the family's measured model rather than leaving the attack model-less (`tools/mobs-to-pack.py`, `PROJECTILE_OVERRIDES`) |
| Damage type, caster energy (30, 4 pips) | assumed by profession |
| Aggro, call for help, leash, respawn (`creatures.json`) | chosen; not in any capture |

The client draws each creature's name from its definition's encoded string, so the labels in the ids
are only for people: "decoded" ones are paired with their name in preforged's loot reports, the others
are the wiki's foe list at that level and profession. Eight skills the creatures were seen using (42,
150, 155, 214, 292, 379, 398, 511) are not in `skills.json` and are left off their bars.

The brain (`crates/asura-world/src/npc/fight.rs`) notices the nearest living player within
`aggro_range`, wakes every idle creature within `assist_range`, chases it with the path finder (no walk
is invented where the finder has no graph), swings, tries its skills every `skill_ms`, gives up past
`leash_range` and walks home to be restored, and lies dead until `respawn_seconds` (0: never, as live).
A hit is noticed wherever it came from: the damaged body engages its attacker even when the attacker
stood beyond `aggro_range`. A `stationary` body (the Plague Worms) never walks: it fights from where it
was placed and lets a player who leaves the farther of its notice range and its swing go. A dead
creature is drawn as a corpse to a client that arrives later.

Ordinary attacks are the engine's (`Engine::attack`): a swing is `0x0035` then `0x00A0` kind 4
(attacker, target) and lands at the **windup**, `interval / 2 - 0.1 s`, with the next swing one full
interval after the last one started (`docs/57-damage-numbers.md`). Kind 4 is measured: the live server
sends it once per swing before the hit (`packets-30352-81363781.jsonl` lines 8508, 8562). The same
capture shows the damage line's skill (`0x009F` kind 10) names the struck agent, not the source; the
wire now does that.
A ranged swing also sends `0x00A4` (projectile launched) at the windup, with the creature's captured
model word (1 a River Skale's spit, 143 an arrow or spear, 343 a Firestarter's fire); a player's bow
shows 143. The shot is closed by `0x00A7` at launch time plus its flight - the same agent, the
`0x00A4` handle and the weapon's damage-type word - hit or miss, and the hit's word follows in the
same frame (Rurik `schema/overrides.json` 167: 311 pairs, mean 7 ms from the launch's own flight).
Without the close the client's projectile never arrives and its next shot is drawn out of step. The
hit lands one flight later, and the flight is the shooter-to-aim distance over `combat.json`'s
`projectile_speed` (1,600 u/s for the bow/spear class): a shot at 100 units arrives in 0.06 s, one
at 1,000 in 0.63 s. Damage is whole points, and a body below one point is dead.

The aim leads the target: at fire it is the target's position plus its velocity over the flight
(GWW "Projectile": the trajectory uses the target's location and velocity at fire). At arrival the
shot connects only if the target stands within `dodge_tolerance` (54 u) ACROSS the line of fire from
the shooter's launch point to the aim and within `dodge_along` (114 u) of the aim along it; outside
that it is a dodge, drawn by `0x00A0` kind 38 reason 1 behind the `0x00A7` with no damage word. A
target that steps sideways dodges; one that runs straight in or straight out stays inside the along
band. Both bands are reconstructions fitted inside retail's zero-violation bands [51.0, 57.5] and
[101, 128] over 572 trusted arrivals (Rurik `toolkit/authsrv/authsrv.py:17340-17375`).
A creature's death is drawn from its health and the DEAD effect: the entry burst now sends property 42
(maximum health) and 34 (the full fraction), so the final hit reaches zero and the death plays. Before
that the client knew neither, and the death flag alone showed nothing.
A player's `GAME_CMSG_ATTACK_AGENT` puts its attack on a foe; nothing stops it but the foe's death or
another target. The server also walks the player into reach (next section).

## Walking to a target

Spacebar or a click on an NPC (`0x8039 INTERACT_LIVING`), on a foe (`0x8026 ATTACK_AGENT`) or on a
party member or another player (`0x8033 INTERACT_PLAYER`) makes the server walk the player to that
agent (`crates/asura-world/src/approach.rs`). What the live server sends, measured:

| what | capture |
|---|---|
| each of the three is answered with `0x002A AGENT_UPDATE_DESTINATION {player, target's position, plane, plane, target}` about 0.1 s later | `movement/movement-live-20261109-200703.jsonl` seq 626 -> 630; `multiplay/multiplay-live-20261106-195109.jsonl` seq 2154 -> 2158; `gwpackets/titles-live-20261106-034507.jsonl` seq 122 -> 124 |
| the client walks itself: no `0x0029` for the player and no position report from it during the walk | same files, the windows after those lines |
| a walking target is announced again about every 0.5 s with its new position; a still one is not | `movement-live-20261109-200703.jsonl` seq 721, 729, 747, 752 (40-unit steps along the NPC's own walk) |
| arrival: `0x0028 AGENT_STOP_MOVING {player}` | seq 637, 5129, 5557 |
| a player already standing at the NPC gets no walk | seq 5260 (the dialog at once) |

The server walks its own copy of the player along the path finder's route, cut where it comes within
range of the target, and steers it on the instance clock (`Instance::steer_approaches`):

- **Interact** (`0x8039`) stops within `interact_range` (`config/Server/movement.json`, 50, chosen:
  the live server still sent a walk at 62 units, seq 5171 -> 5173) and ends there.
- **Attack** (`0x8026`) stops within the weapon's reach (`melee_range` or `bow_range`) and lasts while
  the fight's attack is on that foe; a foe that steps away is chased.
- **Follow** (`0x8033`) stops within `interact_range` and walks again once the target is 1.25 ranges
  away; it lasts until the player moves by itself.

A click, the keyboard or a stop (`0x803E`, `0x803D`, `0x8028`) ends any of them. A target the finder
cannot route to, a map with no path graph, a target that left, or a finished attack ends the walk;
a walk in progress is stopped with `0x0028`.

Not built: what an NPC does once reached (dialog, merchant window: `0x0080`-`0x0081`, `0x007E`,
`0x00C3` in the same capture), and walking into range for a skill (`0x8046`).

## Not built, and what it needs

- **A capture.** One live session using one spell, one attack skill, one enchantment and one hex settles
  the message numbers and order. Nothing else here outranks it.
- **Projectile flight**: Flare, Lightning Javelin and arrows land at completion.
- **Creature loot and experience**: a kill drops nothing and gives nothing.
- **Creature held items**: the captures' `0x0161` for creatures are not copied, so bandits are unarmed
  to look at. `0x0057` model files come from the sanitised client recordings
  (`recordings/client`); 22 creature types have none there, and a type the client cannot draw without
  them shows as a white box.
- **Pets, corpses and minions**: the three unsupported skills.
- **`/resign` deaths are not told to the fight.** A dead player is not revived by anything but a
  skill: no shrine, no return to the outpost.
- **Experience and levels**: stored on the character row and sent in the load block; `/.exp` and
  `/xp <n>` set it, the curve answers the level, and the change is a `0x00EE [0, delta]` award rather
  than a re-sent load block (`docs/54-experience.md`). A kill still awards nothing.
- **Deep Wound's health cap, Weakness and Dazed** are shown but change nothing.
- **Equipment attribute bonuses** (runes, the `542`/`543` words) are not added to effective ranks.
- **Cross-check the catalogue** against Py4GW Stealth for build 39112 (`AGENTS.md`, sources table).
