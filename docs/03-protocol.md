# Protocol

The wire facts the auth service has to reproduce. Everything here is derived from the client's own
protocol and from captures of a working session; where a shape is only known approximately it is
marked **unverified**, and the original code is named so the port can be checked byte for byte instead
of trusted.

Source of truth for every byte below: the original implementation's message tables (`msgdefs.c`), its
handshake (`AuthSrv.c`), travel (`GameSrv.c`), and the captures under `recordings/`.

## 1. Framing

After the handshake, every message in both directions is a small fixed-size structure:

```
u16 header        message id, little endian; bit 15 set means client to server
u8  body[...]     the message's fields, fixed layout per header, and nothing else
```

**There is no length field.** An earlier version of this document claimed a `u16 length` followed the
header. That was wrong, and it is the kind of wrong that costs a session: a port that writes a
length the client does not expect gets back nothing but error `007`. Corrected from the reference:

- `pack_msg` (`msgpack.c`) writes the format's fields and nothing else; `pack_helper` handles
  `TYPE_MSG_HEADER` as exactly two bytes.
- `AuthConnection_SendMessage` (`AuthSrv.c`) is called with `sizeof(struct)` as its input length and
  passes the *written* count back out of `pack_msg`; the written count comes from the format.
- The receive path (`AuthSrv_GetMessages`) reads the header, indexes the format table, and lets
  `unpack_msg` consume the fields. No length is ever read.

So a message's size is a property of its header alone. That table is
`crates/asura-protocol/src/packets.rs`, transcribed once from the original's `msgdefs.c` and
`opcodes.h`, and it carries each message's field layout beside its size so the size is verifiable
rather than trusted. Framing then has only two failure modes: a header that is not in the table
(refuse, as the original server does), and a body that has not arrived yet (wait).

One nuance, and it is a trap: the reference's field vocabulary defines a four-byte header - the u16
header plus a zero u16 (`TYPE_MSG_HEADER_GAME`) - but **no row of either family uses it**
(`crates/asura-protocol/src/packets.rs`, `FieldKind::MsgHeaderGame`). The four-byte headers a sniffer
capture shows are the client's own struct, not this field; section 5e says why.

The whole stream is ARC4-encrypted after the key exchange.

`GWPS_TRACE=1` logs every outgoing message as header, size and the first bytes - that is the
cheapest way to compare a Rust port against the C reference.

## 2. Identifying the connection

The first message a client sends decides what the connection is. It is a bare little-endian `u32`,
**not** a framed message: it has no two-byte header, so decoding it as one would read `0x0400` as a
message id. GWPS compares it against three constants and routes the socket accordingly
(`AuthSrv.c`, `Connection_ReadVersion`):

| header | constant | message | size |
|---|---|---|---|
| `0x000C0400` | `AUTH_CMSG_VERSION_HEADER` | auth: `{header, version, h0008, h000C}` | 16 |
| `0x000C0500` | `GAME_CMSG_VERSION_HEADER` | game: the auth shape plus map/player tokens and both UUIDs | 64 |
| `0x000C0FFF` | `CTRL_CMSG_VERSION_HEADER` | control: the header alone | 4 |

The auth service must make the same decision on its listener, and must reject anything else instead
of guessing.

One fact to keep rather than to re-learn: GWPS stores `version`, `h0008` and `h000C` from the auth
version message and **never compares them to anything**. They are not a compatibility gate.
Reproducing a gate the reference does not have would be inventing protocol; keep the fields for the
packet trace.

## 3. Key exchange

Three messages and one derivation. The sizes and constants below are what
`crates/asura-protocol/src/client/handshake.rs` implements, with the arithmetic pinned by its own
tests (`modpow_is_padded_to_the_full_width` and the exchange round trip).

| step | who | what |
|---|---|---|
| 1 | client -> server | `MSG_CLIENT_SEED`, header `0x4200`, then **64 bytes**: the client's public value as a 512-bit integer, little endian. 66 bytes total, plaintext. |
| 2 | server | `shared = client_public ^ server_private mod prime` - a **512-bit** prime and a 511-bit exponent, both hardcoded in GWPS `main.c` and copied verbatim. |
| 3 | server | draw **20 random bytes**: the master secret. |
| 4 | server -> client | `MSG_SERVER_SEED`, header `0x1601`, then **20 bytes**: `master[i] XOR shared_le[i]`. 22 bytes total, plaintext - it is how the client recovers the master secret. |
| 5 | both | `key = arc4_hash(master)` - 20 bytes, from the client's own construction, which is not a standard hash. |
| 6 | both | ARC4 keyed with those 20 bytes, one instance per direction, both starting at offset zero. Everything from here is scrambled. |

What is **not** on the wire, and matters: no generator, and no server public value. The client's patch
carries the matching parameters, so the server's only arithmetic is raising the client's number to a
fixed exponent. That is why the risk here is transcription, not key management: **the constants must be
identical** or the client fails the handshake and reports nothing but an opaque error.

One deviation, deliberate and recorded (`docs/adr/0009-crypto-primitives.md`): a client public value
congruent to zero modulo the prime yields a zero shared secret, and ASURA refuses the connection
instead of using it. GWPS would carry on with a key that had never been exchanged. The real client
cannot present such a value.

**How this is verified without a client:** the published RC4 vectors, plus fixtures produced by
compiling the reference's `arc4.c` and its `arc4_hash`, plus the exponentiation checked against
CPython's `pow()`. What they cannot prove is that the client accepts the result - that is the packet
trace diff, and it stays M3's acceptance criterion.

## 4. Login and session

1. The client obtains `user_id` and `session_id` (UUIDs) from the portal/web gate, not from us.
2. It sends them in the login message on the auth connection: `AUTH_CMSG_PORTAL_ACCOUNT_LOGIN`
   (`0x8038`, 150 bytes on the wire) carrying `user_id`, `session_id`, a character name and a type.
3. The auth service validates the pair, and answers with the account's character list.
4. The client selects a character; the auth service records the selection.

A login for an account that still holds a session must be resolved deterministically, not by
crashing: GWPS's rule - an incomplete transfer is cleared, a live one is refused - is the behaviour
to keep.

**The portal is not a service here.** In the rig, `user_id` and `session_id` come from a client-side
DLL (`PreSearing/portal/portal.c`) that replaces the client's portal functions and hands it a fixed
pair per test account - there is no HTTP and nothing for this project to serve. What the pair needs
is a matching row, so the database is seeded, not the wire:

| GWPS (`DbSchema.h`, `DbSessionColumnsDef`) | ASURA (`crates/asura-db/migrations/0003_portal_identity.sql`) |
|---|---|
| `session.user_id` | `session.portal_user` |
| `session.session_id` | `session.id` |
| `session.account_id` | `session.account_id` |
| a row must match **both** ids; no row is an auth error, never a crash | `login::find_session(portal_user, session)` returns `None`, and `None` is a refusal |

`tools/seed-dev.ps1` writes the three pairs the DLL knows, and re-running it reopens them; `login` is
`crates/asura-db/src/login.rs`, with the refusal rules under test in `crates/asura-db/tests/login.rs`.

**The portal is an HTTP web gate on loopback, and it *is* something this project has to serve.**
Corrected from a measurement with the real client (build 39112, 2026-10-26). The client launched with
`-portal 127.0.0.1` speaks plain HTTP to `localhost:6601` and gets its identity from four endpoints:

| # | request | reply |
|---|---|---|
| 1 | `GET /Spawned/WebGate/session/create.xml` | `<Reply><Session>UUID</Session></Reply>` - a bearer id for the next calls |
| 2 | `POST /Spawned/WebGate/users/login.xml` (`Authorization: Arena <session>`, body carrying `Provider`, `LoginName`, base64 `Password`, `GameCode`) | `<Reply><UserId>...</UserId>...</Reply>` - the `user_id` of the login message |
| 3 | `POST /Spawned/WebGate/my_account/game_accounts.xml` | the `gw1` account row |
| 4 | `POST /Spawned/WebGate/my_account/token.xml` | `<Reply><Token>...</Token></Reply>` - the `session_id` of the login message |

All four were observed answering `200` from the original web gate,
and the `user_id`/`session_id` it returns are the pair the login message then carries. So the auth
service's scope line - "the web gate: the portal login that hands the client its user_id/session_id"
(`docs/01-scope.md`) - is real work, not a figure of speech. Nothing else about the earlier account of
the portal was wrong (its identity pairs are fixed per test account), but "there is nothing to serve"
was wrong and is corrected here.

**This project serves it: `crates/asura-webgate`.** It is a faithful port of the reference's
`tools/webgate.py` - same four endpoints, same XML, same status codes (`400 who are you?`, `400
Unsupported provider`, `401` without a session) - with one deliberate change: the reference answers
from a hard-coded table of three emails, and this answers from the rows `tools/seed-dev.ps1` writes
through `asura_db::portal::identity_for_email`, so the identity it hands out is the very
`(portal_user, session id)` pair the auth service will resolve. No Python, no reference checkout.

**A login does not spend the session.** GWPS's login path calls `Db_GetSession` and writes nothing,
and the rig presents the same fixed pair on every login - so a service that closed the row on
disconnect or on login would refuse the client's very next attempt. "The newest wins"
(`docs/05-gw2-lessons.md`, rule 1) is a rule about live *connections* and about leases; it is not
licence to consume a row. `login::close_session` and `login::close_other_sessions` exist for the
paths that do end a session deliberately (an explicit logout, a stale sweep) and are deliberately not
called from the login path.

**The reply, in the order GWPS sends it** (`AuthSrv_HandlePortalAccountLogin`):

| # | message | contents |
|---|---|---|
| 1 | `AUTH_SMSG_CHARACTER_INFO` `0x0007`, once per character | request id, character id, name (counted UTF-16, 20 slots), a 64-byte settings block |
| 2 | `AUTH_SMSG_ACCOUNT_SETTINGS` `0x0016` | request id, a **u16** `n_data`, then exactly that many bytes (a 591-byte blob for a new account) |
| 3 | `AUTH_SMSG_FRIEND_STREAM_END` `0x0014` | the request id, twice |
| 4 | `AUTH_SMSG_ACCOUNT_INFO` `0x0011` | request id, the account and selected-character ids, a 200-slot feature array of which 8 are used |
| 5 | `AUTH_SMSG_REQUEST_RESPONSE` `0x0003` | the request id and the status: `0` accepted, `11` (`GM_ERROR_AUTH_ERROR`) refused |

All five are sent, in that order, and `crates/asura-gateway/tests/login.rs` asserts the sequence and the
two fixed sizes over a socket. A refusal sends only row 5.

**Row 2 is the account's own settings, not a constant.** The client uploads its settings on the same
connection with `AUTH_CMSG_SETTING_UPDATE_SIZE` (`0x8021`: request id, total as a `u32`) followed by
one or more `AUTH_CMSG_SETTING_UPDATE_CONTENT` (`0x8020`: the same request id, a `u16` count, and that
many bytes, at most 512). A live client on this server announced 578 bytes and sent 512 + 66; other
sessions upload 454, 455, 457, 579 or 581 bytes, because the blob is the client's UI state - window
positions, visibility and interface options - and it changes when the player moves or closes windows.
`t_account.account_settings` holds the last complete upload; the login answer carries it verbatim, and
the committed 591-byte blob is only the first-login default. `docs/46-ui-settings-persistence.md` is the
evidence and the design.

Two details that are easy to get wrong and are now pinned by tests:

- **The committed settings blob is a copied fact.** For an account that has never uploaded one, the
  client is handed the reference's 591 bytes, and nothing here decodes them. After the first upload the
  account's own bytes replace it, byte for byte.
- **A counted field still occupies its full width.** The feature list in row 4 is an `ARRAY_8` of 200
  slots with a count of 8: the four-byte count, then all 200 bytes. Sizing a message from the count
  instead of the capacity is how a body ends up 192 bytes short.

**UUIDs on the wire are the RFC 4122 little-endian variant** - the first three fields little endian,
the clock sequence and node not (`uuid_enc_le`, GWPS `GmUuid.h`). `uuid::Uuid::to_bytes_le` and
`from_bytes_le` are the conversions; reading them in canonical order swaps the halves of every id, and
the only symptom would be a login that never resolves.

## 5. Travel: the request/redirect cycle

This is the part the whole design exists to protect.

1. **Client asks to travel.** On the game connection, the client sends the district it wants:
   `{map_id, map_type, region, language, district}` with a request id. In GWPS this arrives as
   `CTRL_CMSG`-shaped `StartPlayerTransfer` from the world, and as a direct request message on the
   auth connection at login; both are allocation requests.
2. **the auth service allocates.** It answers only from a world that has the map ready (`docs/02-architecture.md`).
3. **the auth service issues a token.** A fresh player token plus the identity the client's *next* connection
   must present.
4. **The client is redirected.** The response is `TRANSFER_GAME_SERVER_INFO` (`0x01A6`): **GWPS's
   exact struct** (`GameMsg.h:659`, sent as `sizeof(*msg2)`) - **46 bytes, a two-byte header,
   `host[24]`, then five dwords in GWPS's order: `map_token, region, player_token, is_explorable,
   map_id`.** The field order is GWPS's, not the live capture's: `player_token` comes before
   `is_explorable`/`map_id`, and `map_id` is last. Measured against a real client: the captured
   order in a 46-byte frame makes the client read the map id out of the token slot (`007`), and a
   52-byte frame with a four-byte header makes it read `00 00` as the address family (`005`).
   GWPS's bytes are the port target because they are what a working server sends.
   - `host` is 24 bytes: `AF_INET` little endian, port **big** endian, four address bytes, then zero.
   - `is_explorable` describes the **destination** map type - `MapType_MainExplorable |
     MissionExplorable | ArenaExplorable` (1/4/6, `GmMap.h:22-27`) are explorables.
     Deriving it from the source map was a bug that produced client error 005.
5. **A map update follows immediately.** `MAP_UPDATE_CURRENT` (`0x0099`) is sent right after the
   redirect - in live captures the two appear in the same millisecond. 12 bytes:
   `header, map_id, 0`. See the size trap below.
6. **The client reconnects.** It drops the old connection and opens a new one to the address it was
   given, presenting the token. That reconnect is accepted only if it matches the recorded identity;
   a mismatch is client error 007.

### 5c. Is precaching the destination a requirement? (the `--on-demand` experiment)

The reference pre-spawns **one game server per configured map** at startup
(`AuthSrv_PreSpawnMapServers`, `AuthSrv.c:410-449`) so that a travel never constructs anything; its
own comments record why: a map server built *inside* a teleport made the client wait on the loading
screen for the whole construction. This server preloads every hosted map for the same reason - the
`map_config` row, the seat pool row and the path graph are all in place before the first client
connects.

Whether a client actually **requires** the destination to exist before the hop is untested here. That
is what `--on-demand` (or `ASURA_WORLD_ON_DEMAND=1`) measures. With no `--maps` it means **nothing is
preloaded**: the world starts empty and every map is loaded the first time a client names it - at
login (the client's first connection names the character's map) or at `/tp`. No map id is needed on
the command line; that is the whole point: start the server, log in, play, travel.

When a map is named, the demand-load walks the three steps a preload would have done at startup - the
row from `map_config`, the seat pool row in `world_pool`, the path graph out of `Gw.dat` - and logs
each step's cost:

```
world: map 148 'Ascalon City (pre-Searing)' loaded on demand in N ms (row a ms, pool b ms, path graph c ms)
world: "..." teleporting: map 146 -> map 148 ...
```

The one thing an on-demand world publishes up front is a **seat pool row per catalogue map** - a
line in the database saying *"this world can host map X"*, which is what placement reads to decide
where a login may go. It is not an instance and loads nothing: the map's data still arrives when a
client first names it.

The runbook was the experiment itself: the world started with nothing preloaded (the retired
`tools/start-server.ps1 -OnDemand`), then the client logged in and teleported. The verdict is the
client's loading screen plus the world's log lines. **Measured 2026-10-27 on the real client**: with
nothing preloaded, then a loop of teleports
148 -> 163 -> 164 -> 165 -> 166 -> 148 ... - every destination was loaded on demand (`0-2 ms` per
map, with the path-graph step failing fast because no `Gw.dat` was local) and the client took each
hop without an issue. **Precaching is therefore not a requirement on ASURA**; the loading-screen
hang that forced the reference to pre-spawn one server per map (`AuthSrv_PreSpawnServers`) was an
OpenTyria property, not a property of the client or of this world. Give the world a local `Gw.dat`
(`ASURA_GW_DAT` or `data\Gw.dat`) to get the real path-graph cost in that number.

The server half of the experiment is pinned by
`a_connection_to_an_unhosted_map_is_loaded_on_demand` and
`a_travel_to_an_unhosted_map_is_loaded_on_demand` (`crates/asura-world/tests/load.rs`): the
connection or transfer names the unhosted map, its entry and pool row appear when it is demanded,
and the client lands on the destination's own spawn data - a random point of its spawn list for an
outpost, the catalogue row's coordinate when the pack states no list (`docs/22-world-model.md`,
"Spawn points, and the outpost rule").

### 5d. Path graphs without `Gw.dat` (the dump)

The server paths agents from its own copy of each map's walkable geometry: planes, trapezoids,
portals and the split tree the reference's A* (`GmPaths.c`) walks. That geometry lives in the
client's asset archive, `Gw.dat`, as one blob per map - there is no separate map file. Reading the
archive at runtime works but needs 3.6 GB on every server machine, so the graph can be **dumped
once** and loaded instead:

```powershell
# once, with a client archive (ASURA_GW_DAT or --gwdat):
asura-world --dump-path-graphs data\path-graphs.bin
# every start after that, no archive needed:
asura-world            # picks data\path-graphs.bin up automatically
```

- `--dump-path-graphs FILE` writes every map in `map_config` (142 maps at this reading) into FILE;
  `--dump-all` writes every pathing map in the archive instead. `--path-graphs` /
  `ASURA_PATH_GRAPHS` name a dump explicitly; `--gwdat` stays the fallback when no dump exists.
- **Measured 2026-10-27** when the catalogue held twelve maps: 12 of 12 catalogue maps, 486 planes,
  62,326 trapezoids, 11.1 MB - the world then starts with
  `path graphs from the dump data\path-graphs.bin: 12 of 12 map(s) bound` and never opens the archive.
  **The bundled dump today carries 142 maps** (`data/path-graphs.bin`, 87,336,403 bytes, header count
  `0x8e`), so the twelve-map figures above are that day's reading, not the file's size now.
- The format is ours (`crates/asura-world/src/graph_dump.rs`), and it carries exactly what the
  finder consumes. The community `.pmap` files (ACB's Pathingmap Builder, e.g. the archives the
  GW-Pathing-Map visualization projects ship) are *visualization* data: trapezoids and adjacency
  only, no portals or node tree, so they cannot feed the reference's A*. Our dump is produced from
  a client archive once and then travels with the server.
- **The bit-31 trap**: a catalogue blob id may carry the archive's "replacement pending" bit
  (`0x8005EA96` for The Northlands) while an archive binds the plain id (`0x5EA96`). Both are tried
  (`find_path_blob`, the bit-31 trap below), and a substitution is logged.
- The dump is pinned by `crates/asura-world/tests/path_dump.rs`: the walks from a dumped graph are
  compared, waypoint for waypoint, with the walks from the archive import (maps 146 and 148, which
  share one blob), the dump command's own file is loaded back and bound to the catalogue, and the
  **bundled** `data/path-graphs.bin` is walked directly with no archive present.

The dump is derived game data and it **is bundled with this repository**: `data/path-graphs.bin` is
committed (87,336,403 bytes, about 83 MiB, 142 maps), so a fresh clone answers pathfinding without
finding a client archive - the whole server, pathing included, is one clone. The raw `Gw.dat` itself
stays local (3.6 GB, the game's file); `/Gw.dat` and `/data/Gw.dat` are the only ignored archive paths,
and the dump is regenerated with `--dump-path-graphs FILE` when the catalogue or the client changes.

### 5a. The auth-side pair (measured from the reference)

Before any of that, on the **auth** connection, the client asks for a map and the auth service says
where to go. Two fixed-size messages, and one row for each in the generated table:

| message | header | wire size | fields after the header |
|---|---|---|---|
| `AUTH_CMSG_REQUEST_GAME_INSTANCE` | `0x8029` | 26 | `req_id, map_type, map_id, region, district, language` - six dwords |
| `AUTH_SMSG_GAME_SERVER_INFO` | `0x0009` | 42 | `req_id, map_token, map_id, host[24], player_token` |

* `map_token` is the **world's id** in GWPS (`metadata->server_id`), and the game server is what
  decides whether it means anything.
* `player_token` is random per travel (`random_uint32`). It is the same number the auth service hands
  to the world server, so it is the only thing that ties the client's next connection to this one.
* `host[24]` is not a string: 2 bytes of address family (`AF_INET` little-endian, `AF_INET6`
  little-endian - Winsock's 2 and 23, not Linux's 10), then the port **big-endian**, then the address
  bytes, then zero padding. The little/big asymmetry is the reference's own.
* The info message is followed by an ordinary `AUTH_SMSG_REQUEST_RESPONSE` with status 0. A refusal
  is the response with `GM_ERROR_NETWORK_ERROR` (2) **and no info message** - the client then shows an
  error it can describe, where silence is a loading screen that never ends. Sending the info message
  for a port nothing is listening on is worse than either: the client leaves character select and dies
  on a connect timeout.

### 5e. One client, one protocol, two observation layers (corrected 2026-11-01)

**There is one client, one protocol and one wire.** What differs between the captures is the layer the
bytes were read at, and only one of those layers is on a wire:

| layer | what it holds | who reads it |
|---|---|---|
| the client's message struct | a four-byte header (the u16 id and a zero u16), fields at their native width (a u32 where the wire carries a u16), zero padding out to the struct's size, and pointers | the client only. `tools/sniff-capture.py:514` records `entry.size` and `entry.data` from it, which is why every `recordings/client/` file is struct-shaped |
| the wire | a two-byte header and the packed fields, nothing else - what `pack_msg` writes and `unpack_msg` consumes | both ends: GWPS's send log, and this tree's `--capture` and `ASURA_TRACE` runs |
| the table | each header's wire size and field layout | `crates/asura-protocol/src/packets.rs` |

Four measurements, each one checkable:

- **The server that works with this client writes the short form** (`recordings/gwps/world/load-stream.log`,
  `GWPS_TRACE=1`, 2026-10-26): `SEND 0x017C 6 bytes : 7C 01 3F 3F 00 00`, `0x019A` 16 bytes, `0x001E`
  6 bytes, `0x017D` 14 for a five-character name. This tree's rows carry the same 6, 16 and 6, and a
  session that loaded, walked, chatted, travelled and left has the same bytes at the socket
  (`recordings/asura/world/live-client-session-20261027-223528.jsonl`, seq 1-4).
- **The same header through the sniffer is the struct** (`recordings/client/feature/maps/pre-searing-load-3.jsonl`,
  seq 3): `0x017C` at 20 bytes, `7c01 0000 | 3f000000 | 3f000000 | 00000000 00000000` - the extra header
  word, the two ids widened to u32, and eight zero bytes. A `0x000B` in that file is 324 bytes and holds
  absolute addresses (`58fb0806`, `d2ea4001`), which are never on a wire.
- **The deltas are that header word and nothing else**: client-to-server `0x003D` is 28 through the
  sniffer and 26 at this server's socket, `0x003E` 16 against 14, `0x8047` 16 against 14, `0x001E` 8
  against 6, `0x0186` 4 against 2. `docs/21-surrounding-packets.md:28-30` measured the same thing: a
  live row 2 bytes larger than ours is the four-byte header, one 17 larger (`0x0020`) is struct padding.
- **One binary in every capture**: sha256 `dc78cf7f...`, `F:\GW\GW1\Gw.exe`, 10494656 bytes, in the
  header record of every file under `recordings/client/`. There is no second client and no second
  protocol generation.

The rules:

- **A wire size comes from a wire**: `recordings/gwps/world/load-stream.log`, a `--capture` run under
  `recordings/asura/`, or a frame a client accepted under `ASURA_TRACE`. A sniffer capture is evidence
  of a header, a count, an order, a name and a trigger, and never of a size or a field layout.
- **A capture file declares its layer** in its own header record - `"layer":"wire","header_len":2` from
  `asura-world --capture`, `"layer":"struct","header_len":4` from `tools/sniff-capture.py` - and a reader
  takes the header length from there and never assumes it (`tools/capture-read.py` is that reader). The
  same message at the same header is six bytes in one layer and twenty in the other; comparing raw
  records instead of payloads is what manufactured a second protocol out of this difference.
- **A sniffer capture widens every field to four bytes**, so it can never settle a field's *width*: a one-byte
  field and a four-byte field both read `01 00 00 00` there. Sizes and widths come from a wire capture only.
- **A counted field is framed structurally** (the count is on the wire), never by a fixed number.
- **The size in `packets.rs` is the reference's `max_size`**: an upper bound for a counted layout, never
  a length to pad a body out to.
- **A row a wire capture disagrees with is corrected, and the wire capture is cited in the row.** A row
  only a sniffer capture disagrees with is not a defect; correcting it breaks a working stream, which is
  what `docs/27-packet-parity-attempt.md` records.

Two earlier readings of this section are withdrawn, and both cost a session. The first held that one
message set had two wire shapes and this client parsed only one of them; the second held that the
difference was this tree's framing swallowing its own remainder (`0x017C` 6 bytes against 20 read as us
being short, where we were exact). Neither was a second protocol: the first read the struct as the wire,
and the second read a sniffer size as a wire size.

What still stands, and stands because the client was sent the bytes rather than read in a buffer, is
this - movement is where this tree has paid most, so the measurements are kept with it:

| what was sent | what the client did |
|---|---|
| `0x001E` **8 bytes** (`1e 00 00 00` + elapsed), the struct shape | load block played, then **error 007** about 300 ms after the spawn. Server log: `joined map 148` -> `move[spawn]` -> `socket: early eof`, 3 ticks sent |
| `0x001E` **6 bytes** (`1e 00` + elapsed) | loads and plays |
| click answered with `0x002B` (16 B) + `0x0029` **24 B** (trap and plane as dwords), the struct shape | **the client crashed** on the first click |

So the client parses **two-byte headers** with the reference's own sizes: a 6-byte tick, an 18-byte
`0x0029` (`agent, x, y, plane u16, current_plane u16`), `0x0028` for a stop, `0x002C` for a position, and
it has no `0x0025` or `0x002B` at all. The captures stay the authority on *behaviour* - what a click, a
manual walk and a cancel mean, and what the server does with the agent's copy - and those behaviour shapes
are kept in the protocol crate as builders and tests (`agent_move_to_point_live` and friends) so the
difference is recorded rather than remembered. Sending a capture's shape to this client is not a parity
improvement; it is a disconnect (`007`) or a crash.

**One path, two planes: the client asserts.** A walk whose first waypoint is on a different plane from the
agent crashed the client with `Assertion: pos.y >= worldDims.y0`, `P:\Code\Engine\Agent\agint.h(929)` - the
agent's position is validated against the box of the plane it is on. Measured case (map 148, Lakeside blob
`0x1B97D`), click from `(9461.8, 7481.2)` plane 0 to `(8076.0, 6151.5)`:

| waypoint | the dump's own path (`tools/path-oracle.exe` when parity was open; now `src/pathfind.rs` walking `data/path-graphs.bin`) | retail server |
|---|---|---|
| 0 | `(9251.4, 7279.3)` **plane 19**, trap 5506 | never leads with a plane change |
| 1 | `(8426.5, 6487.8)` plane 0, trap 1915 | |
| 2 | `(8076.0, 6151.5)` plane 0, trap 1913 | |

Both this port and the reference produce waypoint 0 **stamped with the plane it enters, in the middle of
that plane and 290 units from the agent**. The retail servers' cross-plane destinations are always portal
corners reached after a same-plane waypoint (`(10860.0, 4532.0)`, which is exactly plane 18's own corner).
So the world hands the client the reference's path minus any **leading** waypoint on another plane
(`begin_walk` in `crates/asura-world/src/agent.rs:147`), and the client walks the straight segment
across the slope itself. The
The dump's own list is unchanged and still checked by `path_dump.rs`; only what reaches the wire is filtered.

### 5f. The two travels a client performs (`0x80C1`, `0x80B1`) and who starts them

**Measured 2026-10-31, and it corrects the first half of this section: a portal crossing is started by
the SERVER, not announced by the client.** In
`recordings/client/feature/travel/login-to-portal-20261031-003720.jsonl` a real client walked out of an
outpost into an explorable and the crossing contained **no client message at all**:

| t | direction | message | what it is |
|---|---|---|---|
| 32.754 | client -> server | `0x003E` `MOVE_TO_COORD` (16 bytes, two floats) | the player walking toward the portal |
| 37.524 | server -> client | `0x00F1`, `0x0028` | the travelling agent, effect `0x100`, then a stop |
| 37.574 | server -> client | `0x01A6` | the redirect: map id **146**, `is_explorable` 1, region `0xFE` |
| 37.625 | server -> client | `0x0099` | the destination |
| 37.981 | server -> client | `0x017C`, `0x0186`, `0x019A` (map 146) | the new instance's load begins |
| 37.982 | client -> server | `0x0091` | its item request |

Zero `0x00C1` in that file. So the world is what decides that the player reached a portal and sends the
same eight-message stream `/tp` and the world map produce (`docs/03` 5f, the eight messages below). The
implementation therefore has a **doorway table**: where a map's doors are, which instance each leads to,
and where a character arriving through one stands. The table is the pack's `portals.json` files - `arches`
state the wall as its two endpoints, `arrivals` the point and facing per map arrived from - and the
measured tables (`crates/asura-world/src/portals.rs`, generated from `data/portals.json`, plus the
`map_portal` rows) answer for a map the pack does not describe. A crossing fires when the step from the
last believed position to this one passes **through** a wall (`crates/asura-world/src/travel.rs`,
`portal_crossed`); a point-shaped doorway still counts within `REACH`. The `0x80C1` arm still names the
touch and starts nothing (`crates/asura-world/src/travel.rs`).

The `0x00C1` messages seen in `captures/zone-ref2-012225.jsonl` are therefore **not** the crossing; they
are the client asking about instances, and what they are owed is still open.

**The world map's request** (`0x80B1`) is the travel the client does start, and this world answers it
(`travel::handle_party_travel`).

| message | header | wire size | fields after the header | what it means |
|---|---|---|---|---|
| `GAME_CMSG_TRAVEL_INIT` | `0x80C1` | 10 | `portal, zero` - two dwords | a client's question about an instance, not a crossing (measured above) |
| `GAME_CMSG_PARTY_TRAVEL` | `0x80B1` | 9 | `map_id` as a `u16`, then three bytes | the player chose an instance on the world map |

**The world clock and this client's own form of `0x80B1`** (`logs/world-8001.log`, a real session:
`world <- 0x80B1 (9 bytes): B1 80 94 00 02 00 00 00 01`):

```
B1 80 | 94 00 | 02 | 00 00 | 00 | 01      instance 148
B1 80 | A6 00 | 02 | 00 00 | 00 | 01      instance 166
```

The destination instance is the first `u16`; the `02` and the trailing `01` were constant across every
send. The retail build carries the same request with a 32-bit instance id
(`b1 00 00 00 | a4 00 00 00 | ... | 01 00 00 00`), so this is one field in two framings, not two messages.
**This world answers it** (`travel::handle_party_travel`): the instance is resolved, and the travel is the
same cycle as `/tp` - reserve, transfer row, `0x01A6` then `0x0099`.

**The eight messages of one travel**, in the order and at the sizes a live server sends them
(`captures/zone-ref2-012225.jsonl`, seq 971 to 982; the same eight in every one of that file's eight
travels):

| # | message | wire size | what it carries |
|---|---|---|---|
| 1 | `0x00F1` `AGENT_UPDATE_EFFECTS` | 10 | the travelling agent, effect word `0x100` |
| 2 | `0x0028` `AGENT_STOP_MOVING` | 6 | the same agent; the walk is over |
| 3 | `0x01A6` `TRANSFER_GAME_SERVER_INFO` | 46 | where to reconnect, and with what |
| 4 | `0x0099` `MAP_UPDATE_CURRENT` | 12 | the destination, in the redirect form |
| 5 | `0x00B0` `PLAYER_UPDATE_PARTY_SIZE` | 12 | `{0, 1}` in the capture |
| 6 | `0x00B1` `PLAYER_UPDATE_PARTY` | 12 | the travelling player, leader `0` |
| 7 | `0x005A` `PLAYER_DESTROY` | 8 | the travelling player |
| 8 | `0x0021` `WORLD_REMOVE_AGENT` | 8 | and their agent |

This world sends all eight, from one `begin_transfer` (`crates/asura-world/src/travel.rs`), for every
trigger - `/tp`, `0x80B1`, and a walk that reaches a doorway in the portal table. The last four withdraw
the travelling
body from the instance it is leaving; the capture's own ids are not ours to reuse, so they are this
connection's player id and agent id.

**The client's instance table** (`0x01A0`) goes out in the load block, ten batches of
`{u16 instance, u32 value}` pairs, which is the message set a live load carries
(`docs/21-surrounding-packets.md`, "The map table", and `game::instance_table` for the data and where it
came from).

Measured in `captures/zone-ref2-012225.jsonl` (the reference checkout's own raw capture of a real client,
not committable - it holds names and UUIDs), 2839 records, 94 of them client-to-server:

* **`0x00C1` fifteen times**, always twelve bytes in the client's struct form and `{small number, 0}`:
  `{2, 0}` at t=26.507, `{7, 0}` at 35.903, `{3, 0}` at 40.265, `{0, 0}` after each. The numbers are
  small (0, 2, 3, 7, 24) while the instances travelled to in the same file are 145, 146, 164 and 165, so
  **the first dword is not a map id**: it names a portal of the instance the character is standing in,
  and where that portal leads is the server's knowledge, not the client's.
* **`0x00B1` twice**, twenty-four bytes in the client's form:
  `b1 00 00 00 | a4 00 00 00 | 00 x4 | 01 00 00 00` - **the destination instance id, first**. At t=20.615
  it named 164 and the redirect four records later went to 164. That is the world-map fast travel.
* Each travel the server then answers is the pair this document already describes: `0x01A6` (52 bytes,
  host[24], then `map_token, region, map_id, is_explorable, player_token, 0` in the client's own field order)
  immediately followed by `0x0099` (12 bytes, the destination), then the new connection's `0x017C`, `0x019A`,
  `0x0196`. **`is_explorable` follows the destination's kind**: 146 and 147 (explorables) carry 1 and 148 (an
  outpost) carries 0, read from the redirects of `recordings/client/feature/travel/doors-20261031-095754.jsonl`
  and `login-to-portal-20261031-003720.jsonl`; `recordings/client/feature/character_creation/maps-tour3.jsonl`
  carries both values for destination 29, so the field is measured on these redirects rather than proven as a
  rule. The eight redirects of the reference checkout's own capture all carry 1 because every one of them was
  a crossing. What the field does to a load is `docs/23-initialization-protocol.md`, "The explorable": at 1,
  against an instance with no population, the client waits 4.3 s before its players block.
* Immediately before each redirect the server sends `0x00F1` `GAME_SMSG_AGENT_UPDATE_EFFECTS` for the
  player's own agent with a dword of `0x100`, and then `0x0028` `AGENT_STOP_MOVING`. So the sequence a
  portal crossing produces is: the walk, the client's `0x80C1`, the player's effect, the stop, the
  redirect.

Two more facts this capture settles, both of which cost a session if forgotten:

* **`0x00B1` is two different messages, one per direction**, and only the direction bit tells them apart:
  client-to-server it is the travel request above; server-to-client it is `PLAYER_UPDATE_PARTY`, twelve
  bytes, sent 29 times in the same file.
* **The reference never handled either one.** `docs/09-parity-audit.md` section 2 lists its 18 game
  handlers and neither is among them, which is why its `/tp` has the documented race: a redirect arrives
  in a map the client never asked to leave.

**What this means for this server.** Both are the cycle `/tp` already implements
(`crates/asura-world/src/travel.rs`: reserve the destination's seat, write the transfer row, send `0x01A6`
then `0x0099`), reached from a different trigger:

* `0x80B1` names the destination itself, so it is a direct call with the client's map id.
* `0x80C1` names a portal of the current instance, so it needs a **portal table**: which portal of which
  instance leads to which instance. That table exists now (`crates/asura-world/src/portals.rs`), keyed by
  position rather than by prop order, so the crossing fires from the walk and the message itself is only
  read (`crates/asura-world/src/travel.rs:154-166`). The portal is a prop, read from the instance's own
  map data in the
  order that data lists it (`docs/22-world-model.md`), so the number the client sends and a table built
  from the archive are the same ordering, and the table can be derived offline, like the path graphs.

**What this world does with `0x80C1` today.** `travel::handle_travel_init`
(`crates/asura-world/src/travel.rs`) names the touch as an event and starts nothing:

```
world: "name" touched a travel object in map 148 'Ascalon City (pre-Searing)': TRAVEL_INIT agent
       0x0000003A, second 0x00000000; no portal row names that agent, so no travel starts
```

That line is the measurement a portal table is built from - which portal, in which instance - and it is
printed without `ASURA_TRACE`, because it is the event, not a packet.

## 6. Traps that cost real time in GWPS


| trap | what happens | rule |
|---|---|---|
| `AUTH_CMSG_SEND_COMPUTER_INFO` sized from the table | the client sends it as **42 bytes** - `u16` counts, variable-length strings, no padding - while the table says 138; framing by the table swallows the message behind it and desynchronises the stream | consume this one structurally (`client::messages::decode_computer_info`), not by table size. Measured against a real client, build 39112: it arrived as 42 bytes of computer info plus a 22-byte `SEND_COMPUTER_HASH` |
| `sizeof(struct)` taken as the wire size | two extra bytes on the wire, so the client misreads every message after it | the size is the format's field sum, not the struct's size. `AuthSrv_SessionInfo` is 14 bytes in C and 10 on the wire - the format has two dwords where the struct has three |
| a length field written where the protocol has none | the client misreads the body as the next header: error `007` | there is no length field (`section 1`) |
| `0x0099` size | 12 bytes in the login batch makes the client fail with 007; the same message is 12 bytes on the redirect and 5 in the login batch | size per context, not per header |
| `is_explorable` from the wrong map | error 005 after travel | always from the destination district |
| `is_explorable` read as a label the client stores | it is a **path selector**: at 1 the client runs its explorable load, and against an instance this world does not fill that load waits 4.3 s before the players block (5138 ms against 849 ms with the field at 0, the same walk and destination) | this world sends **0 for every travel** since 2026-10-31 (`explorable_flag_for`); `ASURA_TRAVEL_EXPLORABLE=auto` sends the destination's own kind, which is live's value, and a slow explorable load is the explorable path asking for an instance this world does not fill - `docs/23-initialization-protocol.md`, "The explorable" |
| settings upload not acknowledged | hang correlated 7/7 with unacknowledged uploads | always answer `AUTH_CMSG_SETTING_UPDATE` with a request response |
| a transfer replayed from a stale queue | client lands in the previous destination | clear the queue after completing one |
| constructing a server during the hop | loading screen forever, zero packets after | the one rule, `docs/02-architecture.md` |
| "chart commands" - chat the **server** generates - sent instead of printed | client crash | print it: the reference returns before the wire for every server-generated answer (`GmChat.c:150-153`, `GmChat.c:209-212`), so printing is the behaviour to match, not a workaround |

Each of these was diagnosed by logging every message at each hop, not by reading code. Keep that
capability: a per-message trace with direction, header, size and first bytes, toggled by an
environment variable.

## 7. File check

The client asks which game files it should have: a 21-byte request answered with 32 bytes. Two runs on
2026-10-25 answered this, and the pair is what the socket layer implements
(`crates/asura-gateway/src/lib.rs`, step 0 of `handle_connection`).

**On the wire.** Captured with `pktmon` filtered to port 6112 and decoded by the run's own pcap parser:
one request, one reply, and the request is the first thing on that socket - nothing precedes it and no
handshake is possible before it. The reply's seven words are six `Gw.dat` file ids in tight consecutive
order with a zero among them, so the question is *which files*, not a transfer. The client treats a
failure here as fatal: with those addresses made unreachable it refused with "Guild Wars is unable to
connect to the ArenaNet network". The captured ids are `391339 391348 344988 391350 391351 0 391349`;
the reference's own literal is `0x100` lower on every id that differs (`client/filecheck.rs`).

**Against a server of ours.** With hosts entries for the six file names, one check arrived on loopback
and was answered - "File check on 0114: answered 32 of 32 bytes" in that run's log.

**What that does not settle.** The file-check address is the client's own choice. It builds and
resolves the file-server names itself, and some of those lookups ignore the hosts file - after flushing
the DNS cache, entries for two of the names reappeared though both were in the file - so the check
reaches our port only when the machine's name resolution sends it there. In the default open mode the
client asks ArenaNet and our handler never fires: that is why it is written as a loop that may run zero
times. A follow-up of four bytes (`F1 0F 04 00`, header `0x40FF1`) comes after the check on that socket;
the reference does not answer it and the client tolerates the silence.

**The seven words are a client-build fact, so this server needs the running client's version.** They are
file ids in the archive the client loaded, so a new client build brings a new set: the reference's
literal and the captured set differ by `0x100` on every id, which is what a rebuild looks like. A server
that answers with another build's ids is answering a question the client did not ask. Nothing here
prevents that from being harmless in practice - and nothing shows that it is, either: the one run in
which a server of ours answered had its other file sockets still going to ArenaNet, so the client may
have been answered elsewhere and never compared ours. What *is* measured is the opposite failure, no
answer at all, and that one is fatal. Until a mismatch is measured one way or the other, treat the set
as required and keep it current.

Getting the current set is a measurement, not a guess, and there are three routes:

1. **Capture the answer the client accepts.** `pktmon` filtered to port 6112, one login in open mode,
   then decode the pcap. This is how the captured set above was obtained, and it is exact by definition -
   it is the bytes the running client was answered with. It repeats once per client build. The pcap
   decoder used then lived outside this tree; `docs/19-running.md` ("Refreshing the file check's seven
   words") has the commands that need no decoder, and what to record beside the set.
2. **Check a set against the archive.** `crates/asura-world/src/gwdat.rs` reads `Gw.dat`'s file table, so
   a set can be confirmed present with sensible sizes before it is trusted. It cannot invent the set:
   which six files are named is the protocol's rule, not the archive's ordering.
3. **Read the set out of the running client.** Py4GW Stealth reads the client through its own image and
   archive code, which is the way to ask a running client what it expects without touching the network.
   Not done here yet, and `tools/sniff-capture.py` will not show it: the sniffer records framed messages
   at the client's handler table, and this request is a bare blob outside that table.

Ghidra answers a different question and is worth it only for that one: what the client actually
compares - the whole 21-byte request, the reply's header and length, or the ids as well - and what it
does when they differ. The ids themselves are not in the code; they come from the archive the client
loaded.

## 8. Coordination between programs: the database, not a protocol

There is no control protocol between the auth service and the world servers for state or placement
(`docs/adr/0004-port-azerothcore-structure.md`). They coordinate through tables:

| table | written by | read by | carries |
|---|---|---|---|
| `world` | each world server | auth service | id, address, port, flags, population, heartbeat |
| `world_pool` | every travel (a reservation), the reaper | travel decisions | per world, per map: capacity and leases |
| `lease` | every travel (a reservation), the reaper | the reaper, the reconciler | one row per reserved slot: world, map, session |
| `session` | auth service | world servers | account, character, issued token |
| `game` schema | world servers | world servers, auth service | characters, items, instance records |

Rules that make this safe rather than hopeful:

- **A reservation is one atomic statement.** `UPDATE world_pool SET leased = leased + 1 WHERE
  world_id = $1 AND map_id = $2 AND leased < capacity RETURNING world_id` either wins a slot or
  matches no row. Two processes cannot both take the last one.
- **A lease is time-bounded.** The reaper expires leases whose owner stopped heartbeating, so a crash
  costs capacity temporarily, never permanently.
- **Every program may fail and re-register.** No state is negotiated live, so there is nothing to
  resynchronise after a restart.
- **The protocol version lives in the schema.** A world server checks the schema version on startup
  and refuses to run against a mismatched database, loudly.

What remains a protocol is the client-facing half: framing, handshake, login and the redirect, all in
the sections above.

## 9. What must be re-proven in Rust

Not "ported and hoped". Each item gets a capture-based test:

0. the message table is the implementation now (its generator and the check were retired when parity closed, docs/16; no
   client needed, so this runs first and stays cheap);
1. handshake completes and the client reaches the character list;
2. login works for a fresh account, and a second login for a signed-in account behaves the same way
   GWPS does;
3. a travel request produces a redirect the client accepts, and the client reconnects;
4. the reconnect is validated, including the rejection path (wrong token -> error, not silence);
5. the map update after the redirect leaves the client in the right map.

## 10. Open questions

- **Why does the client drop the auth connection after its preamble, against this service but not
  against GWPS?** Measured 2026-10-26. Both servers see the same sequence - version, client seed,
  server seed, then the 64-byte preamble - and the client's traffic decrypts correctly in both, so the
  key exchange is right. Against GWPS the portal login follows about nine seconds later (when the
  credentials are submitted) and the session works; against this service the client opened a *second*
  auth connection, sent the same preamble, and then reset it without sending the login. GWPS's own log
  for the working run shows it mis-framed that same preamble (`Unhandled AuthSrv packet with header 35`),
  so the difference is not "GWPS handles it better" - something else about the exchange differs.
  Next evidence to gather: the exact error text the client displays, and a run of this service with
  timestamps and connection ids on every trace line. Both are cheap; neither has been done yet.
- **`AUTH_SMSG_0026` (`0x001A`) declares 32 bytes where its own layout can only occupy 31.** It is a
  `TYPE_NESTED_STRUCT` message, whose length depends on a count carried on the wire, and the
  declared maximum is one byte more than `1 + count * 2` can produce. GWPS would therefore consume
  31 bytes for it while a table-driven framer consumes 32, so a client that sends it would
  desynchronise this port and not GWPS. It is an unnamed friend/social message outside the login
  path, so it is recorded rather than guessed at; `crates/asura-protocol/src/packets.rs` lists
  it in `VARIABLE_LENGTH_HEADERS`. Resolve it when something needs the message.

## 11. The game connection: from placement to a character in a map

After `AUTH_SMSG_GAME_SERVER_INFO` the client opens a **second** connection, to the address the game
was launched with, and presents the tokens it was issued. The reference names its own address in that
message, which is why it never noticed; a client handed a *different* address still dials the launch
address (measured, build 39112), so ASURA's front door relays that connection to the world
(`crates/asura-gateway/src/relay.rs`). Everything below is measured from the reference: the
message layouts from its packed structs (`GameMsg.h`), the sizes from its own send log, and the order
from the functions named beside each step.

### 11.1 How the connection begins

1. **Classify.** The first four bytes are a `u32` and they name the connection kind, exactly as on the
   auth port: `0x000C_0500` is a game connection (`GAME_CMSG_VERSION_HEADER`).
2. **`GAME_CMSG_VERSION`, 64 bytes, fixed.** The client identifies itself and its travel:

   | off | size | field |
   |---|---|---|
   | 0 | 4 | `header` = `0x000C_0500` |
   | 4 | 4 | `version` (the client's build; **never validated**, by us or by the reference) |
   | 8 | 4 | `h0008` (observed 1, never compared) |
   | 12 | 4 | `map_token` - the world id from `GAME_SERVER_INFO` |
   | 16 | 4 | `map_id` |
   | 20 | 4 | `player_token` - the random token issued for this travel |
   | 24 | 16 | `account_uuid` (RFC 4122 little-endian) |
   | 40 | 16 | `character_uuid` (same variant) |
   | 56 | 8 | `h0038`, `h003C` (both 0) |

   The server **sends nothing in reply** to this message. There is no acknowledgement and no error
   message: a token pair the server does not believe in is answered by closing the socket, and the
   client reports that itself as error 007.
3. **Key exchange**, the same as on the auth port: `MSG_CLIENT_SEED` (`0x4200`, 66 bytes) ->
   `MSG_SERVER_SEED` (`0x1601`, 22 bytes), then every byte in both directions is ARC4 with one shared
   keystream.

### 11.2 The token pair is the credential

The reference validates `(map_token, player_token)` plus the character id against what the auth
service recorded when it answered the travel, and closes the connection on any mismatch
(`AuthSrv_DoPostHandshake`). It keeps two pairs per account - *current* and *pending* - so a client
that stayed where it was and a client that took a transfer are both recognised, whichever order the
messages arrive in.

ASURA records that intent in the `transfer` table instead (migration 0004), and `claim_transfer` is a
single `UPDATE ... WHERE claimed_at IS NULL RETURNING`, so two connections presenting the same token
cannot both be believed. A world server checks the character id too: a live token with someone else's
character is refused.

### 11.3 The load block (server -> client, sent unasked)

| # | message | header | size |
|---|---|---|---|
| 1 | `INSTANCE_LOAD_HEAD` | `0x017C` | 6 |
| 2 | `INSTANCE_PLAYER_DATA_START` | `0x0186` | 2 |
| 3 | `INSTANCE_LOAD_PLAYER_NAME` | `0x017D` | `2 + 2 + 2n` |
| 4 | `INSTANCE_LOAD_INFO` | `0x019A` | 16 |

`LOAD_INFO` is `{agent, map_id(u16), is_explorable(u8), district(u32), language(u8), is_observer(u8),
unk(u8)}`. The reference hardcodes `is_explorable = 1` with the comment `// what to put here?`.

### 11.3a The order, as this server sends it, and the capture that shows it

The block goes out unasked the moment the world has the connection, and the order inside it is a rule,
not an accident: a client that receives the same messages in another order stops at the loading screen.
Written down here because it lived only in `crates/asura-world/src/map.rs` (`load_block`), and the box in
`docs/37-rework-layers.md` (layer 7) asked for exactly this.

| # | message | header | what it carries |
|---|---|---|---|
| 1 | `INSTANCE_LOAD_HEAD` | `0x017C` | the load begins |
| 2 | `INSTANCE_PLAYER_DATA_START` | `0x0186` | the player-data block opens |
| 3 | `INSTANCE_LOAD_PLAYER_NAME` | `0x017D` | the character's name |
| 4 | `INSTANCE_LOAD_INFO` | `0x019A` | agent, map id, is-explorable, district, language, observer |
| 5 | `MAP_INIT_START`, `CARTOGRAPHY_DATA` | `0x008A`... | the exploration grid, once per instance |
| 6 | `INSTANCE_TABLE` batches | `0x01A0` | the instances that exist, in the batches a live server sends |

Then the world tick (`0x001E`) and the ping (`0x000C`, `0x000D`) join in, and the client's three requests
are answered in the order they arrive - items (`0x8091`), spawn (`0x8088`), players (`0x8090`) - which is
section 11.4 below. The item stream ends with the six-message download manifest
(`GameSrv_SendDownloadManifest`), whose first `DONE` carries the `MapId_Count` sentinel 877 and whose
second carries the map itself.

**The capture that shows the head of that order:** `recordings/asura/world/live-client-session-20261027-222641.jsonl`,
the first 24 server-to-client messages, recorded by `asura-world --capture` on a live client:

```text
017C 0186 017D 019A 001E 000C 001E 001E 000D 0144 0148 0161 013F 0161 013E 013F 013F 013F 013F 013F 0161 015A 013E 0161
```

The client survived that order (the session continues past it in the same file), which is what makes it
the order to send. Its counts per load - 538 messages for map 166, one `0x017C` and one `0x019A` each -
are in `recordings/asura/world/load-layers-20261031.jsonl`.

### 11.4 The three things the client asks for

The client asks in the order **items (`0x8091`), spawn (`0x8088`), players (`0x8090`)**; the reference
answers whatever arrives, in whatever order.

* **items (`0x8091`, 4 bytes)** -> the item stream opens, the inventory itself, four weapon slots, gold
  storage, the four faction caps, hard mode, character info, `MAP_UPDATE_CURRENT` (5 bytes, the *login*
  shape), `READY_FOR_MAP_SPAWN` (6), and the six-message download manifest.

  The inventory is `GameSrv_SendInventory`: for every bag **in bag-model order** (backpack `0`, material
  storage `5`, unclaimed `6`, storage `7`+, equipped `21`) - and *not* in bag-id order, which is what the
  capture's ids (1, 2, 6, 3, 4, 5) would suggest - first the item that *is* the bag if it has one, then
  `INVENTORY_CREATE_BAG` (`0x013F`, 13 bytes), then each item in it by slot:

  | message | header | wire layout |
  |---|---|---|
  | `CREATE_NAMED_ITEM` | `0x0161` | `u32 item_id, u32 file_id, u8 item_type, u8 dye_tint, u16 dye_colors, u16 materials, u8 unk1, u32 flags, u32 value, u32 model_id, u32 quantity, u16 n_name, u16 name[n], u8 n_modifiers, u32 modifiers[m]` |
  | `ITEM_MOVE_TO_LOCATION` | `0x013E` | `u16 stream, u32 item_id, u16 bag_id, u8 slot` (11 bytes) |
  | `ITEM_SET_PROFESSION` | `0x015A` | `u32 item_id, u8 profession` (7 bytes), sent only when the profession is not 0 |

  `CREATE_NAMED_ITEM` has **no fixed size**, and its two counted fields count differently: the name is a
  `u16` count of UTF-16 units and the modifier list is a **one-byte** count of dwords. That is the
  reference's own packer (`TYPE_NESTED_STRUCT`, `msgpack.c`), and it is why a cupcake's message is 36
  bytes while `sizeof(GameSrv_CreateNamedItem)` is 425. An item name is not a string: the backpack's name
  is a `Gw.dat` template id packed into units (`0x21A8 0xD157 0xB58F 0x166F`), one of which is a lone
  surrogate.
* **spawn (`0x8088`, 2 bytes)** -> `INSTANCE_LOAD_SPAWN_POINT` (`0x0196`), 26 bytes:
  `{map_file_id(u32), position(2xf32), plane(u16), unk0, is_cinematic, unk1[8]}`. These two numbers -
  the Gw.dat blob and the coordinate inside it - are what a client needs to load a map at all.
* **players (`0x8090`, 19 bytes)** -> the long block: PvP unlocks, the account features, the unlocked
  map list, `INSTANCE_LOADED` (the player's team token), skills and attributes, factions, the load
  time, `PLAYER_CREATE` (`0x0059`, `25 + 2n`), `WORLD_CREATE_AGENT` (`0x0020`, **99 bytes, fixed**),
  the agent's health/energy/level/effects/equipment/cape, `WORLD_UPDATE_CONTROLLED_AGENT`, the party
  (create/add/end + pointer), the exploration grid below, and finally **`INSTANCE_LOAD_FINISH`
  (`0x018E`, 2 bytes)** - the message that takes the client off the loading screen.

#### The exploration grid: `0x008B` then `0x008A` (added 2026-10-29, the map-view assertion)

The client keeps one bit per 32-unit cell, **per continent**, and the server is the only source of the
grid's dimensions. A server that sends none leaves them at zero, and the map surfaces then fail their
own consistency check the first time the player opens a map with U or M: client build 39112,

```
Assertion: worldMapDims.x == mapDims.x * DXT_BLOCK_SIZE
P:\Code\Gw\Ui\Game\Map\GmMapView.cpp(1731)
```

measured 2026-10-29 against this server. `mapDims` is the served grid; `worldMapDims` is the
continent's own map art, which the client already holds, and `DXT_BLOCK_SIZE` is 4.

Both messages carry a **game header** - the opcode and a zero `u16`, four bytes - and both were
measured byte for byte from a live client
(`recordings/client/feature/character_creation/character_creation-live-necro_factions.jsonl`,
seq 1055 and 1056):

| # | message | header | wire | layout |
|---|---|---|---|---|
| 1 | `MAP_INIT_START` | `0x008B` | **16** | game header, then `{dims.x(u32), dims.y(u32), compressed byte count(u32)}` |
| 2 | `CARTOGRAPHY_DATA` | `0x008A` | **132** | game header, then a `u32` count of the dwords that follow, then a fixed 124-byte payload; `count * 4` bytes are the blob, the rest is zero |

The captured pair was `8b 00 00 00 | 00 01 00 00 | c0 00 00 00 | 2c 00 00 00` - 256 x 192 cells, 44
bytes - and `8a 00 00 00 | 0b 00 00 00` plus 44 bytes of blob in the same 132-byte message. A
two-byte header, a `u16` count, or a 14-byte `0x008B` each desynchronise the client, which then reads
the following message's header out of the middle of this one (client error `007`, measured twice,
2026-10-29).

The grid is cut into **bands of 16 scanlines**, and each band is a `u16` byte length followed by a
two-state run-length encoding: runs alternate polarity starting with **zeros** (unexplored), a run's
length is the sum of consecutive `0xFF` bytes up to and including the first byte that is not `0xFF`,
and every run after the first is one longer than its bytes say. **A band with nothing explored needs
no bytes at all**: its length is `0`, and the client fills it with unexplored cells. The width must
be a multiple of 32 - the client asserts it while decompressing - and the height a multiple of 16,
because the band format has no partial band.

A live server sends the pair **0.58-0.73 s into the load, immediately after `INSTANCE_LOAD_INFO`,
once per instance** - not at the end of the block. Sending it later is the same as not sending it: the
map view still asserts. (Our first attempt had the messages right and the position wrong, which cost a
session; the timing comes from Rurik's capture corpus, `docs/adr/0011-rurik-is-a-source.md`.)

What this server sends is a grid with **nothing explored**
(`crates/asura-world/src/cartography.rs`): Tyria's measured 64 x 128 cells, 8 empty bands, 16 bytes,
one `0x008A`. The bits are a character's progress and this schema keeps none, so "nothing" is both the
honest value and what a fresh character has.

Open: the dimensions of the other continents. The live client sent 256 x 192 from a character-creation
instance and 64 x 128 from a Tyria outpost, so the grid is per continent. Only Tyria's 64 x 128 is
measured here - a 26-byte blob in the capture - and a map on another continent keeps the assertion it
had before this pair existed. The assertion compares `dims.x * 4` with the continent's own world-map
art, so the width is the number that matters: 64 cells is a 256-texel world map.

### 11.5 The client's own messages, and the one that moves an item

Six of the reference's dispatch cases are answered by a world that has nothing to simulate: the three load
requests above, the ping, the heartbeat, and a disconnect. The two that carry state are movement
(`0x803E`, `0x803D`, `0x8028`, `0x8047`) and the drop:

| message | header | size | layout |
|---|---|---|---|
| `DROP_ITEM` | `0x802C` | 7 | `u32 item_id, u8 count` |
| `ITEM_MOVE` | `0x8072` | 9 | `u32 item_id, u16 bag_id, u8 slot` |
| `EQUIP_ITEM` | `0x8030` | 6 | `u32 item_id` |
| `UNEQUIP_ITEM` | `0x804F` | 6 | `u8, u16, u8` - no item id, and never seen from a live client |
| `POSITION_UPDATE` | `0x803D` | 26 | `Vec2f position, u32 plane, Vec2f unk0, u32 unk1` |

`POSITION_UPDATE` is 26 bytes even though the position is in its first twelve: the framer wants the whole
message, and a port that reads twelve and stops leaves the connection waiting for a message it has already
been given.

`ITEM_MOVE` is the exact inverse of `ITEM_MOVE_TO_LOCATION` (`0x013E`) and is what a live client sent, twice,
when an item was dragged between slots. A move the server accepts is answered with `ITEM_MOVE_TO_LOCATION`
for where the item ended up - with no answer at all the item sprang back and the move did nothing - and a
refused move is answered with the same message naming the slot the item is really in, because the client has
already moved it in its own model.

`EQUIP_ITEM` carries only an item id: which slot it goes in follows from the item's type (`EquippedItemSlot`,
`GmInventory.h`). Wearing something moves the row, takes whatever was worn there off into the first free
carrying slot (and *that* move is echoed, because the client did not make it), and re-sends
`UPDATE_AGENT_VISUAL_EQUIPMENT` - the one thing the client cannot work out for itself.
`UNEQUIP_ITEM`'s three fields hold no item id, so what it means is not known - unequipping by dragging arrives
as `ITEM_MOVE`.

`DROP_ITEM` is answered by **the stack shrinking and nothing else**: the client sends `count = 1` for a stack
of a hundred, so the server takes that many off the row, writes it down, prints it, and sends no message at
all. Three versions have met a live client:

| what the server sent | what the client did |
|---|---|
| `ITEM_REMOVE`, `CREATE_NAMED_ITEM` at what was left, `ITEM_MOVE_TO_LOCATION` | crashed; in a later build the same code cloned the item |
| `WORLD_CREATE_AGENT` for an item agent only | **crashed** - the log's last line is `world -> 0x0020 (99 bytes)` |
| nothing at all | survived, and the stack went down on its own: the client does its own subtraction |

So a drop sends nothing, and there is no switch to turn the other two back on: both candidates are in no
capture, and an *item* agent never can be - the single `0x0020` a capture carries is a **player's** (type 1),
and the one code path that would produce an item agent is the reference's own unfinished drop handler. The
message a working drop needs is a measurement this project does not have yet.

### Chat: what is sent, and what is only printed

`GAME_CMSG_SEND_CHAT_MESSAGE` (`0x8064`) is `{agent_id u32, text String16}`; the first unit of the text is
the channel: `!` all, `#` group, `$` trade, `/` a command, and `@`/`%`/`"` are dropped, as the reference
drops them. A player's line is a **coded string**, not plain text: `CHAT_MESSAGE_CORE` (`0x005D`) carries
the body wrapper `0x0108`, the literal-start marker `0x0107`, the typed units with anything below `0x20`
removed, and the literal-end marker `0x0001`, in fragments of at most 121 units. `CHAT_MESSAGE_LOCAL`
(`0x0061`) then commits it: a `u16` speaker and a `u8` channel, **5 bytes**. That pair is **sent**, exactly
as the reference sends it: `GmChat.c:259-277` is not behind the reference's chat switch.

`GAME_SMSG_CHAT_MESSAGE_SERVER` (`0x005E`) has the same 5-byte shape, with the subject player and channel.
The generated table's two-dword form for both was read as 5 bytes plus the start of another message, and
the client dropped the connection right after the line rendered. Plain text in `0x005D` hits the client's
coded-string assert instead.

The answers the *server* generates are the other half, and they are printed, not sent. Every GM answer and
every "unknown command" in the reference funnels through `GameSrv_SendSystemChatMessage` or
`GameSrv_SendInvalidCommand`, and both return before touching the wire unless `GWPS_CHAT=1`
(`GmChat.c:150-153`, `GmChat.c:209-212`). So the reference's `/maps` answer is a log line; this server's
`/maps` answer is a console line; neither puts a byte on the wire. That is parity, not a switch.

Nothing in this build is behind a switch (#11). The split above is what the reference does, not what this
server enables: a spoken line goes out; a generated answer is printed.

The commands, as the reference's `GmChat.c`/`GmCommands.c` define them: `/stuck` cancels the movement and
broadcasts the agent's position (`0x002C`, **not** `AGENT_STOP_MOVING`, which is what `0x8028` answers);
`/maps` prints the reference's table verbatim - `Teleport targets (N aliases, every one with a map
config):`, then one `  /tp <alias> -> map <id>  <name>` row per map - and sends nothing; `/tp <map>` and
`/teleport <map>` are implemented: the world starts the cycle #5 describes - the seat reserved for the
destination, the transfer row written, `TRANSFER_GAME_SERVER_INFO` then `MAP_UPDATE_CURRENT` sent
(`crates/asura-world/src/travel.rs:181`, reached from the chat command at
`crates/asura-world/src/chat.rs:339`). The alias resolves through the reference's own generated table
(`crates/asura-world/src/travel_targets.rs`), and a target this process cannot serve is refused in the
log, not on the wire. An unknown command prints `Unknown command`, which is the line the reference logs.

Three answers change nothing and send nothing: item id `0`, an item that is not in the backpack/belt
pouch/bag 1/bag 2 (`GmBag_FindItem` looks no further, so storage is not droppable), and a count larger than
the stack. A move is refused when the bag is not the character's, the slot is outside it, or the slot is
occupied - and a refused move is answered with `ITEM_MOVE_TO_LOCATION` for where the item really is, because
the client has already moved it in its own model.

### 11.6 Keeping the session alive

The server sends, without being asked:

* `WORLD_SIMULATION_TICK` (`0x001E`, 6 bytes) at least every 500 ms - a client that stops receiving it
  stops moving;
* `PING_REQUEST` (`0x000C`, 2 bytes) every 5 s, and answers the client's own `PING_REQUEST` (`0x800D`)
  with `PING_REPLY` (`0x000D`, 6 bytes, `{u32 ping}`).

There is **no idle timeout** in the reference and it never disconnects over a missing heartbeat. The
only reliable disconnects are a client `DISCONNECT` (`0x8008`) and a socket error.

### 11.7 Framing on this connection

Same rule as the auth family, different table: a `u16` header (client->server headers carry `0x8000`,
server->client ones do not), and a body whose length is fixed per header except for counted fields,
which carry a `u16` count and then exactly that many units. The two families **collide on header
values** - `0x8000` is `AUTH_CMSG_HEARTBEAT` on the auth connection and `GAME_CMSG_TRADE_ACKNOWLEDGE`
on this one - which is why the one table is keyed by family and why the game lookup takes a direction.

The whole game family is transcribed from the client's protocol: 194 client-to-server headers and 488
server-to-client ones, with each message's field layout
(`crates/asura-protocol/src/packets.rs`, the game family).
