# 12 - The database schema, diffed against the reference's

> **History.** The tooling named below was retired when parity closed (docs/16); nothing here is runnable.

Produced by `tools/parity-schema.py` from `PreSearing/code/OpenTyria/DbSchema.h` and this database;
both that script and the reference checkout are outside the tree now, so the table is the **2026-10-31
reading**, not a live check. Every column the reference declares is either stored here, derived from
another table, or explained; a column that is none of those was the defect this page found, which is how
the character settings block was missed for as long as it was. Two rows have changed since: `last_outpost`
(`crates/asura-db/migrations/0024_last_outpost.sql`) and `eula_accepted`
(`0012_account_eula.sql`) are columns of their own now.

## What the reference declares, and where it lives here

### `session` -> `session`: 4 of 5 column(s) stored

| reference column | state | here |
|---|---|---|
| `user_id` | mapped | `session.portal_user` |
| `session_id` | mapped | `session.id` |
| `created_at` | mapped | `session.started_at` |
| `updated_at` | explained | no analogue: this schema closes a session with `ended_at` and never updates a row in place |
| `account_id` | mapped | `session.account_id` |

Columns this schema has that the reference does not:

- `portal_user` -- the portal identity the rig presents (`migration 0003`); the reference's `user_id`
- `char_id` -- the character this login selected; the reference keeps it on the account
- `world_id` -- the world a travel placed this session on
- `map_id` -- the map that travel asked for
- `ended_at` -- closing a session is what makes a second login win

### `account` -> `account`: 2 of 19 column(s) stored

| reference column | state | here |
|---|---|---|
| `account_id` | mapped | `account.id` |
| `created_at` | mapped | `account.created_at` |
| `updated_at` | explained | no analogue: account rows are written once |
| `eula_accepted` | stored | `account.eula_accepted` (`0012_account_eula.sql`), written when the client accepts (`crates/asura-auth/src/lib.rs:587-601`); the reference sends a hardcoded 24 (`AuthSrv.c:684`) |
| `current_char_id` | explained | kept on `session.char_id` here: the reference puts the last selected character on the account, this scopes it to the login that selected it |
| `current_territory` | explained | unobservable: read into the account row and never sent; a travel here is a `transfer` row |
| `storage_gold` | explained | not stored: `GameSrv.c:825` sends it in the storage-gold message, which this server sends from the character's own gold -- the reference never fills the storage bags either |
| `balthazar_points_max` | explained | fresh-account parity: this server sends the cap the reference sent for an untouched account (10000) in the four `FACTION_MAX_*` messages; the reference reads the column and nothing in it ever writes one |
| `balthazar_points_amount` | explained | zeros are parity: `GameSrv_SendPlayerFactions` reads it, and no handler in the reference assigns one |
| `balthazar_points_total` | explained | zeros are parity, as `balthazar_points_amount` |
| `kurzick_points_max` | explained | fresh-account parity: this server sends the cap the reference sent for an untouched account (10000) in the four `FACTION_MAX_*` messages; the reference reads the column and nothing in it ever writes one |
| `kurzick_points_amount` | explained | zeros are parity: `GameSrv_SendPlayerFactions` reads it, and no handler in the reference assigns one |
| `kurzick_points_total` | explained | zeros are parity, as `kurzick_points_amount` |
| `luxon_points_max` | explained | fresh-account parity: this server sends the cap the reference sent for an untouched account (10000) in the four `FACTION_MAX_*` messages; the reference reads the column and nothing in it ever writes one |
| `luxon_points_amount` | explained | zeros are parity: `GameSrv_SendPlayerFactions` reads it, and no handler in the reference assigns one |
| `luxon_points_total` | explained | zeros are parity, as `luxon_points_amount` |
| `imperial_points_max` | explained | fresh-account parity: this server sends the cap the reference sent for an untouched account (10000) in the four `FACTION_MAX_*` messages; the reference reads the column and nothing in it ever writes one |
| `imperial_points_amount` | explained | zeros are parity: `GameSrv_SendPlayerFactions` reads it, and no handler in the reference assigns one |
| `imperial_points_total` | explained | zeros are parity, as `imperial_points_amount` |

Columns this schema has that the reference does not:

- `username` -- the account name the rig logs in with
- `token` -- the login token the world validates a reconnect against
- `token_at` -- when that token was minted
- `last_ip` -- the address the last login came from

### `character` -> `characters`: 20 of 50 column(s) stored

| reference column | state | here |
|---|---|---|
| `char_id` | mapped | `characters.id` |
| `created_at` | explained | no analogue: rows are created once and `updated_at` carries the only timestamp this schema reads |
| `updated_at` | stored | `characters.updated_at` |
| `account_id` | stored | `characters.account_id` |
| `charname` | mapped | `characters.name` |
| `last_outpost` | stored | `characters.last_outpost` (`0024_last_outpost.sql`), written only for an outpost placement (`asura_db::game::remember_outpost`) |
| `last_guild_hall_id` | explained | zero: nothing here creates a guild, and the reference's own value for a lone character is the null uuid the settings blob carries |
| `sex` | packed | the packed `Appearance` word, `characters.appearance` |
| `height` | packed | the packed `Appearance` word, `characters.appearance` |
| `skin_color` | packed | the packed `Appearance` word, `characters.appearance` |
| `hair_color` | packed | the packed `Appearance` word, `characters.appearance` |
| `face_style` | packed | the packed `Appearance` word, `characters.appearance` |
| `hair_style` | packed | the packed `Appearance` word, `characters.appearance` |
| `race` | packed | the packed `Appearance` word, `characters.appearance` |
| `campaign` | stored | `characters.campaign` |
| `level` | stored | `characters.level` |
| `is_pvp` | stored | `characters.is_pvp` |
| `helm_status` | stored | `characters.helm_status` |
| `skill_points` | explained | zeros are parity: read by `GameSrv_SendPlayerFactions`, written by nothing -- no handler awards experience or points |
| `skill_points_total` | explained | zeros are parity, as `skill_points` |
| `experience` | explained | zeros are parity: read by `GameSrv_SendPlayerFactions`, assigned by no handler in the reference |
| `gold` | stored | `characters.gold` |
| `active_weapon_set` | explained | unobservable: `GameSrv_SendWeaponSlots` sends four zero slots and never reads the column |
| `primary_profession` | stored | `characters.primary_profession` |
| `secondary_profession` | stored | `characters.secondary_profession` |
| `unlocked_skills` | explained | zeros are parity: `GameSrv_SendUpdateUnlockedSkills` sends the bitmap, and the reference's own confirm writes it all-zero |
| `unlocked_maps` | explained | zeros are parity: stored and never read into a message |
| `completed_missions_nm` | explained | zeros are parity: stored and never read into a message |
| `completed_bonuses_nm` | explained | zeros are parity: stored and never read into a message |
| `completed_missions_hm` | explained | zeros are parity: stored and never read into a message |
| `completed_bonuses_hm` | explained | zeros are parity: stored and never read into a message |
| `unlocked_professions` | stored | `characters.unlocked_professions` |
| `skill1` | explained | zeros are parity: `GameSrv_SendSkillbarUpdate` sends them, and every writer in the reference leaves them zero |
| `skill2` | explained | zeros are parity: `GameSrv_SendSkillbarUpdate` sends them, and every writer in the reference leaves them zero |
| `skill3` | explained | zeros are parity: `GameSrv_SendSkillbarUpdate` sends them, and every writer in the reference leaves them zero |
| `skill4` | explained | zeros are parity: `GameSrv_SendSkillbarUpdate` sends them, and every writer in the reference leaves them zero |
| `skill5` | explained | zeros are parity: `GameSrv_SendSkillbarUpdate` sends them, and every writer in the reference leaves them zero |
| `skill6` | explained | zeros are parity: `GameSrv_SendSkillbarUpdate` sends them, and every writer in the reference leaves them zero |
| `skill7` | explained | zeros are parity: `GameSrv_SendSkillbarUpdate` sends them, and every writer in the reference leaves them zero |
| `skill8` | explained | zeros are parity: `GameSrv_SendSkillbarUpdate` sends them, and every writer in the reference leaves them zero |
| `file_id_body` | explained | derived: the settings blob reads the piece's file id from the item in that equipped slot |
| `file_id_legs` | explained | derived: the settings blob reads the piece's file id from the item in that equipped slot |
| `file_id_head` | explained | derived: the settings blob reads the piece's file id from the item in that equipped slot |
| `file_id_boots` | explained | derived: the settings blob reads the piece's file id from the item in that equipped slot |
| `file_id_gloves` | explained | derived: the settings blob reads the piece's file id from the item in that equipped slot |
| `colors_body` | explained | derived: the settings blob reads the dye colours from the item in that equipped slot |
| `colors_legs` | explained | derived: the settings blob reads the dye colours from the item in that equipped slot |
| `colors_head` | explained | derived: the settings blob reads the dye colours from the item in that equipped slot |
| `colors_boots` | explained | derived: the settings blob reads the dye colours from the item in that equipped slot |
| `colors_gloves` | explained | derived: the settings blob reads the dye colours from the item in that equipped slot |

Columns this schema has that the reference does not:

- `pos_x` -- where the character stood, saved on leaving
- `pos_y` -- as `pos_x`
- `pos_plane` -- as `pos_x`

### `bag` -> `bag`: 4 of 7 column(s) stored

| reference column | state | here |
|---|---|---|
| `account_id` | explained | derived: every bag query here is scoped by character, which implies the account |
| `char_id` | mapped | `bag.character_id` |
| `created_at` | explained | no analogue: rows are written once, at creation |
| `updated_at` | explained | no analogue: bags are not updated |
| `bag_model_id` | stored | `bag.bag_model_id` |
| `bag_type` | stored | `bag.bag_type` |
| `slot_count` | stored | `bag.slot_count` |

Columns this schema has that the reference does not:

- `bag_id` -- the number the client is told; per character, as the wire wants it
- `assoc_item_id` -- the item that *is* the bag (`bag_item_id` in the reference's in-memory model)

### `item` -> `item`: 10 of 14 column(s) stored

| reference column | state | here |
|---|---|---|
| `account_id` | explained | derived: every item query here is scoped by character, which implies the account |
| `char_id` | mapped | `item.character_id` |
| `bag_model_id` | explained | stored as `item.bag_id`, a reference to the bag row rather than a copy of its model id |
| `slot` | stored | `item.slot` |
| `created_at` | explained | no analogue: rows are written once, at creation |
| `updated_at` | explained | no analogue: an item is rewritten, never dated |
| `quantity` | stored | `item.quantity` |
| `dye_tint` | stored | `item.dye_tint` |
| `dye_colors` | stored | `item.dye_colors` |
| `item_type` | stored | `item.item_type` |
| `profession` | stored | `item.profession` |
| `model_id` | stored | `item.model_id` |
| `file_id` | stored | `item.file_id` |
| `flags` | stored | `item.flags` |

Columns this schema has that the reference does not:

- `materials` -- `CREATE_NAMED_ITEM`'s materials field
- `unk1` -- `CREATE_NAMED_ITEM`'s unexplained field, kept so the message can be rebuilt
- `value` -- `CREATE_NAMED_ITEM`'s value
- `name` -- the item's name, as the UTF-16 units the wire carries
- `modifiers` -- the item's modifier list, as the wire carries it

## What this does not claim

The database is the least observable surface this project has, and a column being stored here is not
the same as a feature working. What the check does claim is narrower and worth more: no column of the
reference's schema has been forgotten, and every one that is absent is absent for a recorded reason.

The two mappings a reader should look at first are `session.user_id` -> `session.portal_user` (the
portal identity the client presents, which the reference calls `user_id`) and `character.charname` ->
`characters.name` (UTF-16 units on the wire, text here).
