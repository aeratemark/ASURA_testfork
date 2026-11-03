# Selectable self-contained game profiles

Owner's design decision, 2026-11-02. No loader, launcher, database change, or profile data is
implemented by this document.

## Intention

ASURA is an offline server whose first content target faithfully reproduces the base game.
The owner also wants other setups: earlier game-era content/rules, extended campaigns, and
custom stories using the same world and server engine.

A future launcher selects what to load and starts the server. The current work must support
that selection without requiring the launcher to exist first. The startup option and on-disk
packaging format are not chosen yet; do not invent a launcher UI as part of loader support.

## Chosen model: one self-contained profile

The owner selected all-or-nothing content loading. Load one selected profile for a server
launch. Do not merge its content with a base profile or search other profiles for missing data.

The owner's example is explicit: if a profile contains three quests, it has three quests.
It does not acquire the rest of the base game's quests automatically.

The intended authoring workflow is to copy the base profile, modify that copy, and share the
resulting profile. Adding three quests to that copy distributes the copied base content plus
the additions. The copy does not retain an inheritance relationship to its original.

| Profile example | Result |
|---|---|
| Base profile | Its installed base-game content |
| A profile containing three quests | Those three quests, with no base-quest fallback |
| A copy of the base profile with three quests added | The copied base quests and the three additions |
| A copy of the base profile with quests removed | The remaining quests; removed quests are not restored from elsewhere |

Self-contained means no implicit content inheritance. It does not mean every profile must
reproduce the entire base game or duplicate the Rust executable, pathing assets, or other
shared engine resources. Required content references must still resolve within that profile
or to an explicitly supported engine resource.

## Boundaries for implementation

- Keep the engine separate from the selected game content. Rust runs the mechanics; the
  profile supplies the supported definitions, relevant settings, and Lua behavior.
- Keep the faithful base profile identifiable. Editing a custom copy does not edit the base.
- Load only the selected profile's definitions and scripts. Do not scan all installed profiles
  for extra quests, scripts, or replacements.
- Keep installed database content consistent with the selected profile. Selecting another
  profile must not leave previous-profile quests or NPCs visible by accident.
- The exact database separation mechanism is unresolved. Separate databases, schemas, and
  profile-keyed rows are not interchangeable implementation assumptions; decide before writing
  the affected storage/loading code.
- A missing required reference is a validation error. Missing optional content stays absent.
  Do not fill gaps with base content, guessed records, or successful no-op handlers.
- Selecting a profile happens for startup. Live profile switching and mixed profiles inside
  one running server are not part of the chosen simple loading model.
- Keep runtime progress and transient state distinct from distributable content. The owner
  selected separate account settings, characters, levels, and progress for each profile;
  do not share them across profiles.
- New mechanics still need engine support. Selecting profiles does not authorize a Rust
  plugin ABI, arbitrary native modules, or a new client protocol/version.

This model does not need overlays, patch stacking, profile dependency graphs, load-order conflict
resolution, or merge rules. Do not add them to support the author's three-quest example.

## Chosen model: separate experiences and saved state

The owner clarified: each mod has its own account settings and is a separate experience, with
separate characters, levels, and progress. Profile selection therefore selects both the game
content and that profile's account/player state.

Keep account settings, character records, levels/experience, inventory/equipment, quest history,
unlocks, and other durable gameplay progress scoped to the selected profile. Runtime sessions
and instance assignments must resolve against that same profile. Selecting another profile
must not reuse the previous profile's character or its state.

Switching back to an existing profile resumes its own persisted experience under that game's
normal login rules. It does not reset that profile merely because a different one was loaded
in between. Do not add cross-profile character transfer or automatic save conversion.

This decides logical isolation, not the physical database layout. Whether to use separate
databases, schemas, or profile-scoped records still needs an explicit implementation design.
Sharing the server executable or database service does not authorize sharing player state.

## Control panel and future launcher

The loader needs an explicit selected-profile input that the future launcher can supply.
The flag/path name and profile metadata format remain to be designed.

The control panel's content editor must identify which profile it edits. If it is showing a
running server, distinguish the profile being edited from the one currently loaded. Saving
content does not claim that a live server switched profiles or reloaded content.

Copying/exporting a base profile must mean copying distributable content and its scripts, not
cloning account credentials, active sessions, leases, or a player's private progress.
The export/copy mechanism and format are unresolved. JSON was previously recommended for
content exchange, but the owner has not selected it; Lua is chosen for behavior, not implicitly
for all profile configuration.

## Decisions still needed

1. What is the profile's distributable representation and how does the database install/read it?
2. Which storage layout implements the chosen separation of content and account/player state?
3. How is a profile identified and checked against the engine features/version it requires?

Resolve these before dependent implementation. Do not reopen the selected no-inheritance model
or build the future launcher to answer a loader/storage question.

## Future acceptance cases

- Starting the base profile loads its own supported content.
- Starting a valid three-quest profile exposes only those three quests.
- Starting a copied-and-extended base profile exposes the copied content and additions.
- A removed optional content record does not reappear through fallback.
- Switching the startup selection does not mix content from the previous profile.
- Account settings and characters changed in profile A do not change those in profile B.
- A character's level, inventory, quest progress, and unlocks remain local to its profile.
- Returning to profile A restores its own persisted state rather than profile B's state or a reset.
- Invalid required references are reported without silently completing missing content.
- A profile copy leaves the original and private operational/player data untouched.

These are acceptance requirements for later implementation, not test results.
