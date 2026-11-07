# 46 - The interface layout is account settings, and where the client saves it

The UI is not saved by a game-server message. The client uploads its whole settings blob on the auth
connection with `AUTH_CMSG_SETTING_UPDATE_SIZE` and `AUTH_CMSG_SETTING_UPDATE_CONTENT`, and the server
answers the next login with that blob in `AUTH_SMSG_ACCOUNT_SETTINGS`. ASURA already received the
uploads and acknowledged them; until migration 0032 it threw the bytes away and answered every login
with the same committed 591-byte blob, so every session began from the same layout.

## 1. What the client sends, measured

The evidence is the server's own auth log, `logs/auth.log`, from live client sessions against this
server (build 39112). The client sniffer records the game channel; the settings upload travels on the
auth connection, so this log is where it appears.

```
auth <- 0x8021 AUTH_CMSG_SETTING_UPDATE_SIZE 10 bytes: 05 00 00 00 42 02 00 00
auth <- 0x8020 AUTH_CMSG_SETTING_UPDATE_CONTENT 520 bytes: 05 00 00 00 00 02 08 00 66 21 00 10 38 43 7f de
auth <- 0x8020 AUTH_CMSG_SETTING_UPDATE_CONTENT 74 bytes: 05 00 00 00 42 00 00 06 23 01 00 00 7b 01 00 00
```

- `0x8021` carries `req_id` (u32) and the total (u32). Here request 5 announces `0x0242` = 578 bytes.
- `0x8020` carries the same `req_id`, a u16 count, then exactly that many bytes. The two messages above
  carry 512 and 66 bytes: 512 + 66 = 578.
- The blob starts `08 00 66 21 00 10 38 43 7f de ...`, the same first bytes as the committed
  `config/Server/account_settings.json` blob, and then diverges: it is the client's own state, not a
  copy of what it was sent.

The same log shows the blob changing length between sessions: 454, 455, 457, 578, 579 and 581 bytes
were all uploaded by live clients on this machine. A fixed message cannot change length; a layout can.
Three different accounts on the same client produced three different final blobs (578, 455 and 579
bytes), so the state is per account, not a per-machine constant. Inside one session the blob also
changes: one session uploaded 579 then 455 bytes, and another alternated 578 and 454 as windows were
opened and closed. The client uploads again after the layout changes, which is what makes the last
upload the state to keep.

## 2. Why this is the UI layout

- The client's own Options / Interface tab says "Your interface options will be saved with your account
  on the server" (the panel in the reported screenshots). F11 opens that panel and also puts the
  interface into the move/resize mode; double-clicking an element restores its default.
- The client-side name for the payload is the game settings array: Py4GW's `GW::ui::GetSettings` returns
  `Context::GetGameSettingsAddress()` as a `GWArray<unsigned char>`, and its live measurement was
  **798 bytes** in one session (`Py4GW_Stealth-master/docs/UIMANAGER_PORT.md`, the `GetSettings` row).
  The array's length varies with the state, which is what the upload keeps.
- GWCA's `GW::UI::WindowPosition` is `{u32 state; Vec2f p1; Vec2f p2}`, `state & 1` is visible, and
  `GetWindowPosition`/`SetWindowPosition` address it by `WindowID` (`Dependencies/GWCA/include/GWCA/Managers/UIMgr.h`,
  `WindowPosition` and `WindowID`). The array is the in-memory form of the same state the blob carries.
- The auth message is account-level, not character-level: it is sent once per account login, before a
  character is selected. The character-select blob is the separate `AUTH_CMSG_UPDATE_CHARACTER_SETTINGS`
  (`0x8009`), which ASURA already answers without storing because the character row holds that state.

## 3. The client-side trigger

The client's Preferences module (`P:\Code\Gw\Pref\PrApi.cpp`, read by Rurik) holds `s_windows`,
`s_dirtyFlags`, `s_delay` and `s_flushDelay`, and the area-load message
`GAME_SMSG_PREFERENCES_BLOB` (`0x00B5`) is handled by that same module
(`Rurik-main/studies/newopcodes/FINDINGS.md`, the `0x00B5` entry). The auth log shows repeated
uploads within one session, which is what a dirty-flag flush looks like; the exact delay is the
client's, not this server's.

ASURA does not send `0x00B5` today (`docs/24-stream-parity.md` lists it as a stream-parity gap). That
message is a separate, constant 50-byte blob in the live captures, and its content is not decoded. The
account settings upload is the state the client actually saves, and it is the one this change keeps.

## 4. What changed in this tree

- `crates/asura-db/migrations/0032_account_settings.sql` adds `t_account.account_settings bytea`.
- `crates/asura-db/src/account.rs` reads the column with the account row and writes it with
  `record_settings`.
- `crates/asura-protocol/src/client/messages.rs` reads the two upload messages:
  `decode_settings_size` and `decode_settings_content`, with the 512-byte chunk cap.
- `crates/asura-auth/src/lib.rs` accumulates the chunks per connection, stores the blob when the
  announced total arrives, and answers `AUTH_SMSG_ACCOUNT_SETTINGS` with the account's stored blob.
  A new account, or an upload that is empty or larger than the 2048-byte field, falls back to
  `config/Server/account_settings.json`.
- `crates/asura-gateway/tests/login.rs` proves the loop over a socket: log in, upload 578 bytes, log in
  again, and the settings message carries those exact bytes.

The bytes are never decoded. The server does not know which byte is a window position, and does not
need to: the client produced the blob, and the client is the only reader that matters.

## 5. How to see it work

1. Apply the migration (`asura-supervisor schema`) and restart the stack (`asura-supervisor start`).
2. Log in. The first login still gets the committed default, because the account has not uploaded yet.
3. Move or close interface windows. The client uploads the new blob; the auth log shows `0x8021` and
   `0x8020` and then `auth: stored ... account settings for ...`.
4. Log out and back in. The same layout is restored: the login answer is the last blob the client
   uploaded.

If the layout is still reset after step 4, the remaining suspect is `0x00B5`: capture the area load
with the client sniffer and compare the 50-byte blob's payload with `GetSettings`, then decide whether
the game server must send it. That is a separate measurement, not a guess.
