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
- **World map:** the leader in an outpost starts a countdown (`0x01BC`, guessed layout), then the party
  goes; anyone else - or anyone in an explorable - leaves the party and travels alone.
- **Arrival:** the first member builds the party from the ledger, the others show as not in the map until
  they arrive (as live sends it); one that never arrives is dropped after the grace time.
- **Merges:** an invitation may name a party of more than one; the merge leader is the inviter on equal
  sizes, otherwise the larger party's leader.
- **Disconnect hold:** a member that drops in an explorable stays in the party, grey, for 15 minutes;
  logging back in puts it back in the party's map.
- **Storage:** migration `0030_parties.sql` (`t_parties`, `t_party_members`); run **Create tables
  (schema)** once. **Values:** `config/World/Vanilla/party_travel.json`.
- Not built: `/resign` and the wipe return (last), cancelling the countdown, one explorable per party.

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

### Other changes

- **Outposts load as outposts:** `INSTANCE_LOAD_INFO` now sends `is_explorable 0` for a town (it sent 1
  for every map), as the live server does (`crates/asura-world/src/map.rs`).
- **Capture window:** `tools/sniff-capture-gui.py` runs `tools/sniff-capture.py` from a window (Start,
  Stop, a list of the title records found). Needs the 32-bit Python 3.13. `sniff-capture.py` gained
  `--stop-file` for the Stop button.
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
| [`client-patcher/README.md`](client-patcher/README.md) | patching the client |
| [`docs/03-protocol.md`](docs/03-protocol.md) | the protocol |
