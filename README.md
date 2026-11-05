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
