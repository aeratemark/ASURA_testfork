# A.S.U.R.A. - experimental pre-Searing fork

A private Guild Wars 1 server (Rust + PostgreSQL) that only runs pre-Searing. This is an experimental fork:
things change fast and some features are half-built on purpose.

## Run it

1. Install Rust and PostgreSQL.
2. Double-click `run_server.py`. On the Setup tab: **Run all setup steps** the first time; after that
   **Build** (code changed), **Create tables (schema)** (a new migration), **Add test accounts and maps
   (seed)** (the seed file changed). Then **Start server** on the Server tab.
3. Patch the client with `client-patcher/` and log in with an email from `tools/seed-dev.sql`
   (`user1@example.com` ... `user3@example.com`), or with your own from `tools/seed-local.sql`. The
   password is not checked.

Logs are in `logs/`. A config file in `config/Server/` is read once at start: restart the server after
changing one.

## How it fits together

```
client --portal login--> web gate  :6601  (who am I?)
client --login--------> auth       :6112  (key exchange, session, character)
client --gameplay----> auth :6112 --relays--> world (instance server) :6113+
                                   everything durable --> PostgreSQL
```

| crate | what it does |
|---|---|
| `asura-auth` | login, characters, travel decisions; relays the game connection |
| `asura-world` | the maps: outposts, explorables, movement, items, chat, titles |
| `asura-webgate` | the portal login the client asks before it logs in |
| `asura-supervisor` | starts and watches the services (`config/services.toml`) |
| `asura-protocol` | the client's messages, byte for byte |
| `asura-rules` | game decisions as pure functions over values handed in |
| `asura-config` | reads `config/Server/*.json` |
| `asura-db` | the database, its migrations and queries |

## What this fork changed (2026-11-05)

### Titles

The Hero panel's Titles tab works, with retail's own data. Design and evidence:
`docs/40-titles-system-research.md` (section 13 says where every value comes from).

- **Wire:** `crates/asura-protocol/src/game/title.rs` - the four title messages (`0x00F3` rank record,
  `0x00F4` shown rank, `0x00F5` point update, `0x00F6` track) and the show request (`0x8058`).
- **Rules:** `crates/asura-rules/src/titles.rs` - which rank a point count is on, what a load or a point
  change sends, whether a title may be shown, and checks for the client's crash rules.
- **Content:** `config/Server/titles.json` - one entry per title, no title in code. Today: Tyrian
  Cartographer, Drunkard, Survivor, Kind of a Big Deal, Legendary Defender of Ascalon, Sweet Tooth.
  Rank names are encrypted strings; one that is not captured yet is `0101` (empty).
- **Storage:** migration `0028_titles.sql` - `t_character_titles`, `t_account_titles`, and
  `t_characters.display_title` (the title a character shows).
- **World:** `crates/asura-world/src/titles.rs` - the title block in the load, a shown title in
  introductions, the Hero panel's show/hide (`0x8058`/`0x8059`), and `add_points`/`set_points`, the calls
  an earning rule makes.
- **Cartographer** earns from exploration: the share of pre-Searing's map you have explored (this
  server's own rule - retail does not count pre-Searing).
- **`/title <name or id> <points>`** sets a title's points for testing, e.g. `/title ldoa 20`,
  `/title drunkard 10000`. The answer is in the world log.

### Party travel (2026-11-06, experimental)

A party moves as one. Design and evidence: `docs/45-party-travel.md`.

**Status: highly experimental, far from finished.** The rules below are what is built; the list at the
end is what is not, and a case no capture covers is a guess.

- **Doorways:** any member crossing brings the whole party, to the door's arrival point.
- **World map:** the leader in an outpost starts a measured countdown (`0x01DA` then `0x01BC`, 10 s,
  "travelling to <map>"), then the party goes; anyone else - or anyone in an explorable - leaves the
  party and travels alone. The leader cancels with `0x80A3`, answered by the unlock `0x01DA {0, 0}`.
- **Members left behind:** a member without the destination unlocked stays; the leader is asked first
  with `0x01E7 {5}`, and a confirmed request moves the rest at once, without a countdown.
- **Individual travel:** an unconfirmed world-map request is answered with `0x01E7 {4}`, the client's
  "you will leave your party" popup; the confirmed request travels alone, and the members that stay
  keep its name greyed.
- **Arrival:** the first member builds the party from the ledger, the others show as not in the map until
  they arrive (as live sends it); one that never arrives is dropped after the grace time.
- **Merges:** an invitation may name a party of more than one; the merge leader is the inviter on equal
  sizes, otherwise the larger party's leader.
- **Disconnect hold:** a member that drops in an explorable stays in the party, grey, for 15 minutes;
  logging back in puts it back in the party's map.
- **`/resign`:** a chat line, not a message. Each member dies after `resign_secs` (5 s) with `0x002D` +
  `0x0026 {4}`; the last death sends `0x01D9`, and the leader's `0x80A7` returns the party to its last
  outpost.
- **Storage:** migration `0030_parties.sql` (`t_parties`, `t_party_members`) and `0031` (a doorway
  transfer's source map); run **Create tables (schema)** once. **Values:**
  `config/World/Vanilla/party_travel.json`.
- Not built: one explorable per party, heroes and henchmen, the members' side of a cancel, and `0x01DA`
  before a solo travel or a defeated party's return.

### Guilds (2026-11-06, experimental)

The guild panel and its registration window work from `/guild create`. Design and evidence:
`docs/41-guild-system-research.md`.

**Status: highly experimental, far from finished.** Only the batches the two captures show are
answered; everything else is a guess, and a live client decides.

- **Wire:** `crates/asura-protocol/src/game/guild.rs` - builders for `0x0118`, `0x011C`, `0x0121`,
  `0x0127`, `0x0128`, `0x012F`, `0x0131`, `0x0132`, `0x0133` and the registration window; readers for
  the four requests (`0x8036` create, `0x80B5` invite, `0x80BB` leave, `0x80BE` announcement).
- **Rules:** `crates/asura-rules/src/guild.rs` - who may create, invite, leave and announce.
- **World:** `crates/asura-world/src/guild.rs` - the guild block in the load, a guilded peer's badge
  right after its player create, and the four requests.
- **`/guild create`** opens the Guild Registration window in place of a Guild Registrar NPC (owner,
  2026-11-06). The window's own messages are a guess: no capture shows it.
- **Storage:** migration `0029_guilds.sql` (`t_guilds`, `t_guild_members`, `t_guild_invites`); run
  **Create tables (schema)** once. **Values:** `config/Server/guilds.json` - the bytes a new guild's
  `0x0121` carries, and the `0x0133` counter's start and step.
- **Free for now:** no gold message (`0x014F`) is sent for a creation or an invitation; the two prices
  in the config are read and logged only.
- Not built: the invitee's `0x0123` and the accept, kicks, promotions and demotions, guild chat, capes,
  guild halls, and the roster beyond the founder's own row.

### Movement, doorways and maps (2026-11-07)

- **Manual movement:** a reported direction is traced through connected walkable cells
  (`crates/asura-world/src/pathfind/directional.rs`, `PathGraphs::walk_direction`) instead of routing to
  a point on a distant plane; the distances are `config/Server/directional_movement.json`. Provisional:
  the owner reports better movement but not perfect, and this is not client-confirmed.
- **Doorways:** the pack's `portals.json` is the only source - the compiled tables, `map_portal` rows and
  SQL fallback are gone. A wall triggers on a step that crosses or touches it, endpoints included, and
  every simulated step is tested; a point doorway uses a box edge (`config/Server/portals.json`). The
  last finite client report is kept separately from simulation ticks, so a clamped or refused report no
  longer loses a crossing.
- **Arrivals:** a doorway transfer writes its source map to `t_transfer.arrival_from_map_id` (migration
  `0031`); a saved coordinate alone no longer selects an arrival. Outposts 165 and 166 name 161 and 162
  as their source, not themselves.
- **Maps:** the pack's `basic.json` `map_file_id` outranks `t_map_config` (Northlands 147 is 116025,
  fixing the load crash), and every map carries `name_id`, the client's `AreaInfo` string id
  (`tools/add-map-name-ids.py`); the party countdown needs it. Fort Ranik (166) moved from `Missions/` to
  `Outposts/`.
- **Cartography:** `ASURA_CARTOGRAPHY=full` now sends a captured 105-byte retail stream (2,140 blocks)
  instead of the synthetic fill the client rejects (`crates/asura-world/src/cartography.rs`).

### UI settings persistence (2026-11-07)

- **The client saves its interface state on the auth connection.** `AUTH_CMSG_SETTING_UPDATE_SIZE`
  (`0x8021`) announces the blob's total, then one or more `AUTH_CMSG_SETTING_UPDATE_CONTENT`
  (`0x8020`) carry it in chunks of at most 512 bytes. The live client on this server announced 578
  bytes and sent 512 + 66; the same log shows 454-581-byte blobs, and one session uploading 579 then
  455 as windows were closed. The blob starts `08 00 66 21`, the same prefix as the committed
  default, then carries the player's own state. Evidence: `docs/46-ui-settings-persistence.md`.
- **Storage:** migration `0032_account_settings.sql` adds `t_account.account_settings bytea`.
  `crates/asura-db/src/account.rs` reads it with the account row and writes it with
  `record_settings`.
- **Wire:** `crates/asura-protocol/src/client/messages.rs` decodes the two upload messages
  (`decode_settings_size`, `decode_settings_content`).
- **Auth:** `crates/asura-auth/src/lib.rs` accumulates the chunks, stores the blob when the announced
  total arrives, and answers `AUTH_SMSG_ACCOUNT_SETTINGS` (`0x0016`) with it at the next login. A new
  account, or an upload outside `1..=2048` bytes, falls back to
  `config/Server/account_settings.json` (the reference's 591-byte default). The
  `AUTH_CMSG_UPDATE_CHARACTER_SETTINGS` (`0x8009`) character-select blob is still acknowledged only;
  the character row holds that state.
- **Test:** `crates/asura-gateway/tests/login.rs::the_clients_settings_upload_is_returned_at_the_next_login`
  logs in, uploads 578 bytes, logs in again, and reads the same bytes back.
- **Owner-confirmed:** a full account logout/login now keeps the layout; before this change the
  client was handed the same committed blob on every login, so the inventory bags returned to their
  default position.
- **Not done:** `GAME_SMSG_PREFERENCES_BLOB` (`0x00B5`, a constant 50-byte Preferences blob at area
  load) is still unhandled; `docs/24-stream-parity.md` lists it as a stream-parity gap. The account
  settings blob is the state the client actually saves; what `0x00B5` carries is not decoded.

### Other changes

- **Outposts load as outposts:** `INSTANCE_LOAD_INFO` now sends `is_explorable 0` for a town (it sent 1
  for every map), as the live server does (`crates/asura-world/src/map.rs`).
- **Capture window:** `tools/sniff-capture-gui.py` runs `tools/sniff-capture.py` from a window. A
  capture-type dropdown sets the folder under `recordings/client/feature/` and its header filter
  (cartography, titles, movement, travel, maps, chat, items, guild, multiplay), and **Go** starts at
  once (Stop ends it cleanly). Needs the 32-bit Python 3.13.
- **Own logins:** `tools/seed-local.sql` (not committed, `.gitignore`) holds a machine's own accounts in
  `seed-dev.sql`'s shape; `asura-supervisor seed` applies it after `seed-dev.sql` when it exists.
- **Docs:** doc 40 corrected (the `0x0059` it counted as a title hide is the server's `PLAYER_CREATE`)
  and extended with every live-client result.

### Open

- Titles: rank names still missing for Survivor 2, Sweet Tooth 2, Kind of a Big Deal 3 and 5,
  Cartographer 2-6, and all of Party Animal; they come from captures. No earning yet for Drunkard,
  Sweets, Party, Survivor, LDoA or Kind of a Big Deal. Showing a title under your name is being tested.
- A brand-new account with no character cannot create one (login refuses travel without a selected
  character, as the reference does); the seed gives every account one character as a workaround.
- No live capture has a title being ticked or unticked yet, so the server's reply to hiding a title is
  unknown and nothing is sent.

## Read more

| document | what it settles |
|---|---|
| [`AGENTS.md`](AGENTS.md) | the binding rules for anyone (or any agent) changing this repo |
| [`docs/HANDOFF.md`](docs/HANDOFF.md) | the evidence behind those rules and where work stands |
| [`docs/19-running.md`](docs/19-running.md) | how to run it, and troubleshooting |
| [`docs/40-titles-system-research.md`](docs/40-titles-system-research.md) | the title system |
| [`docs/46-ui-settings-persistence.md`](docs/46-ui-settings-persistence.md) | the account settings blob and the saved interface layout |
| [`client-patcher/README.md`](client-patcher/README.md) | patching the client |
| [`docs/03-protocol.md`](docs/03-protocol.md) | the protocol |
