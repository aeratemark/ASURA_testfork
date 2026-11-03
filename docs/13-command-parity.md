# 13 - The GM command surface, diffed against the reference's

> **History.** The tooling named below was retired when parity closed (docs/16); nothing here is runnable.

Produced by `tools/parity-commands.py` from `GmCommands.c`, `GmChat.c` and `GmMaps.h` against this
repo's command table and its alias table; the script is gone from the tree, so this is the 2026-10-31
reading rather than a live check. The command surface is small and its blast radius is not: `/tp`
moves a player between servers, `/maps` is the only way to find out what a world hosts, and `/stuck`
sends a different message from the cancel that looks like it. All three are implemented
(`crates/asura-world/src/chat.rs`, `crates/asura-world/src/travel.rs:181`).

## The commands

| what a player types | where the reference handles it | takes an argument | what this server answers |
|---|---|---|---|
| `/tp`, `/teleport` | `GmCommands.c:184 GameSrv_TryHandleGmCommand -> GameSrv_HandleTeleportCommand` | yes | a transfer, or the reference's refusals |
| `/maps` | `GmCommands.c:206 GameSrv_TryHandleGmCommand -> GmCommand_ListTargets` | no | the alias table in the log |
| `/stuck` | `GmChat.c:243 GameSrv_HandleChatCommand` | no | the agent's position |

The reference's names, from its two dispatch points: `/tp` (GmCommands.c), `/teleport` (GmCommands.c), `/maps` (GmCommands.c), `/stuck` (GmChat.c).

## The `/tp` aliases

`s_teleport_targets` is twenty aliases over twelve maps: two for most, because the reference lets a
tester type `lakeside` or `lakeside county`. This server resolves through the same generated table,
so every one of them works -- which it did not until the table was generated: matching on each
map's *name* accepted six and answered "unknown map name" for the rest.

| alias | label | map | type | hosted here |
|---|---|---|---|---|
| `ascalon` | Ascalon City (pre-Searing) | 148 | 0 | yes |
| `ascalon city` | Ascalon City (pre-Searing) | 148 | 0 | yes |
| `lakeside` | Lakeside County | 146 | 1 | yes |
| `lakeside county` | Lakeside County | 146 | 1 | yes |
| `ashford` | Ashford Abbey | 164 | 0 | yes |
| `ashford abbey` | Ashford Abbey | 164 | 0 | yes |
| `catacombs` | The Catacombs | 145 | 1 | yes |
| `the catacombs` | The Catacombs | 145 | 1 | yes |
| `northlands` | The Northlands | 147 | 1 | yes |
| `the northlands` | The Northlands | 147 | 1 | yes |
| `green hills` | Green Hills County | 160 | 1 | yes |
| `barradin` | The Barradin Estate | 163 | 0 | yes |
| `barradin estate` | The Barradin Estate | 163 | 0 | yes |
| `wizards folly` | Wizard's Folly | 161 | 1 | yes |
| `foibles` | Foibles Fair | 165 | 0 | yes |
| `foibles fair` | Foibles Fair | 165 | 0 | yes |
| `regent` | Regent Valley (pre-Searing) | 162 | 1 | yes |
| `regent valley` | Regent Valley (pre-Searing) | 162 | 1 | yes |
| `fort ranik` | Fort Ranik | 166 | 0 | yes |
| `kamadan` | Kamadan, Jewel of Istan | 449 | 0 | yes |

`hosted` is this server's answer to "can we actually serve that map": the reference refuses a
target with no `GmMapConfig` rather than letting its loader fall back to Kamadan, and this refuses a
map the world was not started with. Both refusals are logged, and neither puts an answer on the
wire -- the reference's chat gate is off in both servers, so a GM answer has always been a log
line.
