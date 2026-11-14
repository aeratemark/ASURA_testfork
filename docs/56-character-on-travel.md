# 56 - The character a travel moves, and the one the client presents

**Status, 2026-11-13:** fixed, awaiting a client run. A doorway from Foibles Fair to Wizard's Folly
served the account's Monk to a player who was playing its Warrior.

## 1. What happened

The world writes a transfer row for every travel, naming the character it is moving
(`crates/asura-world/src/travel.rs`, `begin_transfer` -> `placement::begin_transfer`). The client then
opens a new game connection and its `GAME_CMSG_VERSION` message presents a character id. That id is not
always the character being moved: at login it is the character the player just chose on the selection
screen, but on a travel it can still carry the login selection - measured 2026-11-13, the account's
Monk while the player was its Warrior.

The world preferred the presented character whenever it was on the account
(`crates/asura-world/src/connection.rs`), which is right for a login and wrong for a travel. The map
load then described the Monk: its name, inventory, level and position. The Warrior's seat was released
with the travel, so nothing looked like an error in the log except the character id.

## 2. The fix

A transfer now says whether it is a **login handoff** or a **world travel**:

- `t_transfer.is_login` (migration 0037), false by default;
- the auth service passes `true` (`crates/asura-auth/src/travel.rs`);
- every world travel passes `false` (`crates/asura-world/src/travel.rs`).

`connection::served_character` decides:

| transfer | presented character | served |
|---|---|---|
| login | on the account | the presented one, as before |
| login | absent, foreign, or deleted | the transfer's |
| travel | anything | **the transfer's**, always |

The travel case logs the disagreement at world level:

```text
world: travel token 0x... moves character <warrior>; the client presented <monk> and the transfer wins
```

A travel is not a choice: the world wrote the row for the character whose seat, movement, party and
map the message is about. A login is a choice, because the client makes it after the auth service has
written the handoff.

## 3. Tests

- `connection::tests::a_travel_serves_the_character_the_transfer_moves` - a travel presents the
  account's Monk and still serves the Warrior.
- `connection::tests::a_login_may_use_the_character_the_client_selected_when_it_is_owned` - a login
  uses the presented character when it is owned, and falls back otherwise.
- `crates/asura-world/tests/load.rs` writes every transfer as a travel (`false`), which is what its
  harness exercises.

## 4. Acceptance

1. Play a Warrior, cross Foibles Fair -> Wizard's Folly.
2. The loading screen and the character sheet stay the Warrior; the world log says
   `travel token ... and the transfer wins` only if the client presented another id.
3. Log out, select the Monk at character select, enter: the Monk is served, as a login choice.
