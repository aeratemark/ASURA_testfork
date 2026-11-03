# 24 - Stream parity: every message a live client exchanged, and what we do with it

Status: maintained by hand from the committed captures. Its generator was retired with the rest of the
parity tooling (`docs/16-implementation-plan.md`); a capture that adds a header adds a row here.

The owner's rule, 2026-10-31: "100% replicate the stream without discriminating a single message, even
if that has nothing to do with our task". So every header a live capture contains is listed with what this
tree does about it, and no row is dropped because a feature does not need it.

Header values are written with the direction bit set the way the wire carries it (`0x80xx` from the client,
`0x00xx` from the server); the sniffer records the client's without it, which the script adds back.

**`sizes seen` is the client's message-struct size, never a wire size** (`docs/03-protocol.md` 5e). It
runs 2 bytes over the wire on every fixed message (`0x001E` 8 against 6, `0x0186` 4 against 2) and much
further where the struct pads (`0x000A` 240 against 140, `0x000B` 324 against 88). The column says "what
the client held": it is a count and a shape, not a length to send. The wire size per header is
`crates/asura-protocol/src/packets.rs`, and the wire forms to copy are
`recordings/gwps/world/load-stream.log` and the sessions under `recordings/asura/`.

| header | direction | live count | sizes seen | our name | our status |
|---|---|---|---|---|---|
| 0x000C | S -> C | 111 | 4 | GAME_SMSG_PING_REQUEST | Emitted |
| 0x000D | S -> C | 111 | 8 | GAME_SMSG_PING_REPLY | Emitted |
| 0x000F | S -> C | 160 | 16 | GAME_SMSG_ACCOUNT_FEATURE | Emitted |
| 0x0018 | S -> C | 6 | 20 | GAME_SMSG_PVP_UPDATE_UNLOCKED_HEROES | Emitted |
| 0x001A | S -> C | 435 | 20/28 | GAME_SMSG_PVP_ITEM_STREAM_ADD | Emitted |
| 0x001D | S -> C | 6 | 260 | GAME_SMSG_PVP_UPDATE_UNLOCKED_SKILLS | Emitted |
| 0x001E | S -> C | 4644 | 8 | GAME_SMSG_WORLD_SIMULATION_TICK | Emitted |
| 0x0020 | S -> C | 386 | 116 | GAME_SMSG_WORLD_CREATE_AGENT | Emitted |
| 0x0021 | S -> C | 195 | 8 | GAME_SMSG_WORLD_REMOVE_AGENT | Emitted |
| 0x0022 | S -> C | 8 | 12 | GAME_SMSG_WORLD_UPDATE_CONTROLLED_AGENT | Emitted |
| 0x0025 | S -> C | 369 | 20 | GAME_SMSG_AGENT_UPDATE_DIRECTION | Emitted |
| 0x0026 | S -> C | 7 | 12 | GAME_SMSG_AGENT_UPDATE_FLAGS | Observed |
| 0x0027 | S -> C | 2 | 12 | GAME_SMSG_AGENT_UPDATE_SPEED_BASE | Known |
| 0x0028 | S -> C | 36 | 8 | GAME_SMSG_AGENT_STOP_MOVING | Emitted |
| 0x0029 | S -> C | 1535 | 24 | GAME_SMSG_AGENT_MOVE_TO_POINT | Emitted |
| 0x002A | S -> C | 10 | 28 | GAME_SMSG_AGENT_UPDATE_DESTINATION | Known |
| 0x002B | S -> C | 255 | 16 | GAME_SMSG_AGENT_UPDATE_SPEED | Emitted |
| 0x002E | S -> C | 112 | 16 | GAME_SMSG_AGENT_UPDATE_ROTATION | Known |
| 0x002F | S -> C | 1 | 12 | GAME_SMSG_AGENT_UPDATE_ALLEGIANCE | Known |
| 0x0035 | S -> C | 4 | 16 | - | Unnamed |
| 0x0037 | S -> C | 33 | 16 | GAME_SMSG_AGENT_UPDATE_ATTRIBUTE_POINTS | Emitted |
| 0x003C | S -> C | 115 | 16 | GAME_SMSG_PLAYER_UPDATE_FLAGS | Emitted |
| 0x003E | S -> C | 1 | 8 | GAME_SMSG_AGENT_ALLY_DESTROY | Known |
| 0x0048 | S -> C | 61 | 12 | GAME_SMSG_AGENT_DISPLAY_CAPE | Emitted |
| 0x004C | S -> C | 35 | 520 | GAME_SMSG_QUEST_DESCRIPTION | Known |
| 0x0053 | S -> C | 12 | 24 | GAME_SMSG_QUEST_ADD_MARKER | Known |
| 0x0056 | S -> C | 152 | 52 | GAME_SMSG_NPC_UPDATE_PROPERTIES | Known |
| 0x0057 | S -> C | 131 | 24 | GAME_SMSG_NPC_UPDATE_MODEL | Known |
| 0x0059 | S -> C | 65 | 92 | GAME_SMSG_PLAYER_CREATE | Emitted |
| 0x005A | S -> C | 4 | 8 | GAME_SMSG_PLAYER_DESTROY | Emitted |
| 0x005D | S -> C | 1 | 248 | GAME_SMSG_CHAT_MESSAGE_CORE | Emitted |
| 0x005E | S -> C | 1 | 12 | GAME_SMSG_CHAT_MESSAGE_SERVER | Emitted |
| 0x0067 | S -> C | 1 | 16 | - | Unnamed |
| 0x006B | S -> C | 13 | 12 | GAME_SMSG_POST_PROCESS | Known |
| 0x006D | S -> C | 312 | 16 | GAME_SMSG_NPC_UPDATE_WEAPONS | Known |
| 0x006E | S -> C | 61 | 44 | GAME_SMSG_UPDATE_AGENT_VISUAL_EQUIPMENT | Emitted |
| 0x006F | S -> C | 1 | 16 | GAME_SMSG_UPDATE_AGENT_VISUAL_EQUIPMENT_SLOT | Observed |
| 0x007C | S -> C | 32 | 8 | - | Unnamed |
| 0x007D | S -> C | 33 | 8 | - | Unnamed |
| 0x008A | S -> C | 33 | 132 | GAME_SMSG_CARTOGRAPHY_DATA | Emitted |
| 0x008B | S -> C | 33 | 16 | GAME_SMSG_MAP_INIT_START | Emitted |
| 0x008C | S -> C | 39 | 16 | - | Unnamed |
| 0x008D | S -> C | 1 | 48 | GAME_SMSG_MAP_MARKER | Inferred |
| 0x0099 | S -> C | 255 | 12 | GAME_SMSG_MAP_UPDATE_CURRENT | Emitted |
| 0x009A | S -> C | 28 | 12 | GAME_SMSG_AGENT_UPDATE_SCALE | Known |
| 0x009B | S -> C | 191 | 72 | GAME_SMSG_AGENT_UPDATE_NPC_NAME | Known |
| 0x009C | S -> C | 33 | 12 | GAME_SMSG_AGENT_UPDATE_MORALE | Emitted |
| 0x009F | S -> C | 394 | 16 | GAME_SMSG_AGENT_PROPERTY_UPDATE_INT | Emitted |
| 0x00A0 | S -> C | 12 | 20 | GAME_SMSG_AGENT_PROPERTY_UPDATE_INT_TARGET | Known |
| 0x00A2 | S -> C | 30 | 16 | GAME_SMSG_AGENT_PROPERTY_UPDATE_FLOAT | Emitted |
| 0x00A3 | S -> C | 10 | 20 | GAME_SMSG_AGENT_PROPERTY_UPDATE_FLOAT_TARGET | Known |
| 0x00A4 | S -> C | 1 | 36 | GAME_SMSG_AGENT_PROJECTILE_LAUNCHED | Known |
| 0x00A5 | S -> C | 1 | 252 | GAME_SMSG_AGENT_UPDATE_SPEECH_BUBBLE | Known |
| 0x00A6 | S -> C | 92 | 16 | GAME_SMSG_AGENT_UPDATE_PROFESSION | Known |
| 0x00A7 | S -> C | 40 | 16 | - | Unnamed |
| 0x00AA | S -> C | 2 | 16 | GAME_SMSG_AGENT_CREATE_NPC | Known |
| 0x00AE | S -> C | 3 | 12 | GAME_SMSG_AGENT_UPDATE_MODEL | Known |
| 0x00B0 | S -> C | 82 | 12 | GAME_SMSG_PLAYER_UPDATE_PARTY_SIZE | Emitted |
| 0x00B1 | S -> C | 80 | 12 | GAME_SMSG_PLAYER_UPDATE_PARTY | Emitted |
| 0x00B5 | S -> C | 32 | 4 | GAME_SMSG_PREFERENCES_BLOB | Inferred |
| 0x00B6 | S -> C | 33 | 12 | GAME_SMSG_PLAYER_UPDATE_UNLOCKED_PROFESSIONS | Emitted |
| 0x00B7 | S -> C | 37 | 20 | GAME_SMSG_PLAYER_UPDATE_PROFESSION | Emitted |
| 0x00BA | S -> C | 7 | 4 | GAME_SMSG_MISSION_STREAM_START | Known |
| 0x00CF | S -> C | 9 | 12 | - | Unnamed |
| 0x00D0 | S -> C | 1 | 8 | - | Unnamed |
| 0x00DA | S -> C | 33 | 44 | GAME_SMSG_SKILLBAR_UPDATE | Emitted |
| 0x00DB | S -> C | 38 | 260 | GAME_SMSG_UPDATE_UNLOCKED_SKILLS | Emitted |
| 0x00E9 | S -> C | 33 | 64 | GAME_SMSG_CHARACTER_UPDATE_FACTIONS | Emitted |
| 0x00EF | S -> C | 33 | 12 | GAME_SMSG_CHAR_VISIBILITY_FLAGS | Emitted |
| 0x00F0 | S -> C | 352 | 12 | GAME_SMSG_AGENT_INITIAL_EFFECTS | Emitted |
| 0x00F1 | S -> C | 37 | 12 | GAME_SMSG_AGENT_UPDATE_EFFECTS | Known |
| 0x00F2 | S -> C | 32 | 8 | GAME_SMSG_INSTANCE_LOADED | Emitted |
| 0x00F3 | S -> C | 931 | 32 | GAME_SMSG_TITLE_RANK_DATA | Known |
| 0x00F4 | S -> C | 36 | 12 | GAME_SMSG_TITLE_RANK_DISPLAY | Known |
| 0x00F6 | S -> C | 442 | 76 | GAME_SMSG_TITLE_TRACK_INFO | Known |
| 0x00FE | S -> C | 1 | 4 | GAME_SMSG_CINEMATIC_SKIP_EVERYONE | Known |
| 0x0100 | S -> C | 2 | 8 | GAME_SMSG_CINEMATIC_START | Known |
| 0x0103 | S -> C | 1 | 4 | GAME_SMSG_CINEMATIC_DATA_END | Known |
| 0x0104 | S -> C | 2 | 4 | GAME_SMSG_CINEMATIC_DATA | Known |
| 0x0107 | S -> C | 15 | 84 | - | Unnamed |
| 0x010E | S -> C | 10 | 16 | GAME_SMSG_MANIPULATE_MAP_OBJECT | Known |
| 0x0111 | S -> C | 9 | 16 | GAME_SMSG_MANIPULATE_MAP_OBJECT2 | Known |
| 0x0115 | S -> C | 22 | 12 | GAME_SMSG_GADGET_SET_FLAGS | Inferred |
| 0x0118 | S -> C | 12 | 28 | GAME_SMSG_GUILD_PLAYER_ROLE | Known |
| 0x011A | S -> C | 18 | 124 | GAME_SMSG_TOWN_ALLIANCE_OBJECT | Known |
| 0x011B | S -> C | 1 | 12 | GAME_SMSG_TOWN_ALLIANCE_COMMIT | Inferred |
| 0x011C | S -> C | 9 | 4 | GAME_SMSG_TOWN_ALLIANCE_END | Inferred |
| 0x011F | S -> C | 60 | 136 | - | Unnamed |
| 0x0121 | S -> C | 43 | 160 | GAME_SMSG_GUILD_GENERAL_INFO | Known |
| 0x0127 | S -> C | 45 | 272 | GAME_SMSG_GUILD_PLAYER_INFO | Known |
| 0x012E | S -> C | 12 | 8 | GAME_SMSG_LOCATION_TAG | Inferred |
| 0x012F | S -> C | 3 | 556 | - | Unnamed |
| 0x013A | S -> C | 83 | 72 | GAME_SMSG_ITEM_UPDATE_CUSTOMIZED_NAME | Known |
| 0x013E | S -> C | 2322 | 20 | GAME_SMSG_ITEM_MOVED_TO_LOCATION | Emitted |
| 0x013F | S -> C | 239 | 28 | GAME_SMSG_INVENTORY_CREATE_BAG | Emitted |
| 0x0144 | S -> C | 39 | 12 | GAME_SMSG_ITEM_STREAM_CREATE | Emitted |
| 0x0148 | S -> C | 39 | 12 | GAME_SMSG_ITEM_SET_ACTIVE_WEAPON_SET | Emitted |
| 0x014B | S -> C | 15 | 20 | GAME_SMSG_ITEM_CHANGE_LOCATION | Known |
| 0x014D | S -> C | 10 | 12 | GAME_SMSG_ITEM_REMOVE | Emitted |
| 0x015A | S -> C | 61 | 12 | GAME_SMSG_ITEM_SET_PROFESSION | Emitted |
| 0x015B | S -> C | 25 | 12 | - | Unnamed |
| 0x015E | S -> C | 279 | 40 | GAME_SMSG_ITEM_LOW_DETAIL | Observed |
| 0x0161 | S -> C | 2672 | 176/180/184/188/192/196/200/212/216/228 | GAME_SMSG_CREATE_NAMED_ITEM | Emitted |
| 0x0168 | S -> C | 6 | 12 | - | Unnamed |
| 0x016E | S -> C | 11 | 8 | GAME_SMSG_LATENCY_ENABLE | Inferred |
| 0x017C | S -> C | 256 | 20 | GAME_SMSG_INSTANCE_LOAD_HEAD | Emitted |
| 0x017D | S -> C | 37 | 44 | GAME_SMSG_INSTANCE_LOAD_PLAYER_NAME | Emitted |
| 0x0186 | S -> C | 39 | 4 | GAME_SMSG_INSTANCE_PLAYER_DATA_START | Emitted |
| 0x0188 | S -> C | 1 | 64 | GAME_SMSG_CHAR_CREATION_SUCCESS | Emitted |
| 0x0189 | S -> C | 2 | 4 | GAME_SMSG_CHAR_CREATION_START | Emitted |
| 0x018E | S -> C | 11 | 4 | GAME_SMSG_INSTANCE_LOAD_FINISH | Emitted |
| 0x0194 | S -> C | 11 | 8 | - | Unnamed |
| 0x0196 | S -> C | 254 | 36 | GAME_SMSG_INSTANCE_LOAD_SPAWN_POINT | Emitted |
| 0x0197 | S -> C | 7 | 4 | GAME_SMSG_INSTANCE_MANIFEST_DATA | Known |
| 0x0198 | S -> C | 5 | 16 | GAME_SMSG_INSTANCE_MANIFEST_DONE | Emitted |
| 0x0199 | S -> C | 6 | 8 | GAME_SMSG_INSTANCE_MANIFEST_PHASE | Emitted |
| 0x019A | S -> C | 254 | 32 | GAME_SMSG_INSTANCE_LOAD_INFO | Emitted |
| 0x01A0 | S -> C | 30 | 28/132 | GAME_SMSG_INSTANCE_TABLE | Emitted |
| 0x01A6 | S -> C | 248 | 52 | GAME_SMSG_TRANSFER_GAME_SERVER_INFO | Emitted |
| 0x01AE | S -> C | 21 | 64 | - | Unnamed |
| 0x01B3 | S -> C | 10 | 12 | GAME_SMSG_CHARACTER_UDPATE_PARTY | Emitted |
| 0x01BE | S -> C | 11 | 8 | - | Unnamed |
| 0x01BF | S -> C | 11 | 8 | GAME_SMSG_PARTY_SET_DIFFICULTY | Observed |
| 0x01CC | S -> C | 10 | 16 | GAME_SMSG_PARTY_PLAYER_ADD | Emitted |
| 0x01D3 | S -> C | 9 | 8 | GAME_SMSG_PARTY_CREATE | Emitted |
| 0x01D4 | S -> C | 10 | 8 | GAME_SMSG_PARTY_MEMBER_STREAM_END | Emitted |
| 0x01DA | S -> C | 6 | 256 | GAME_SMSG_PARTY_LOCK | Known |
| 0x01E0 | S -> C | 8 | 8 | GAME_SMSG_PARTY_SEARCH_SEEK | Known |
| 0x800A | C -> S | 4 | 240 | GAME_CMSG_COMPUTER_INFO | Known |
| 0x800B | C -> S | 4 | 324 | GAME_CMSG_COMPUTER_HASH | Known |
| 0x800C | C -> S | 32 | 4 | GAME_CMSG_HEARTBEAT | Handled |
| 0x800D | C -> S | 34 | 4 | GAME_CMSG_PING_REQUEST | Handled |
| 0x8012 | C -> S | 12 | 8 | GAME_CMSG_QUEST_REQUEST_INFOS | Known |
| 0x803D | C -> S | 442 | 28 | GAME_CMSG_POSITION_UPDATE | Handled |
| 0x803E | C -> S | 1 | 16 | GAME_CMSG_MOVE_TO_COORD | Handled |
| 0x8047 | C -> S | 31 | 16 | GAME_CMSG_LAST_POS_BEFORE_MOVE_CANCELED | Handled |
| 0x8051 | C -> S | 3 | 12 | GAME_CMSG_INTERACT_GADGET | Known |
| 0x8060 | C -> S | 6 | 12 | GAME_CMSG_CHAR_CREATION_CHANGE_PROF | Handled |
| 0x8063 | C -> S | 1 | 4 | GAME_CMSG_SKIP_CINEMATIC | Known |
| 0x8084 | C -> S | 35 | 12 | GAME_CMSG_ITEM_UPDATE_EQUIPPED_COLOR | Known |
| 0x8088 | C -> S | 36 | 4 | GAME_CMSG_INSTANCE_LOAD_REQUEST_SPAWN | Handled |
| 0x8089 | C -> S | 2 | 4 | GAME_CMSG_CHAR_CREATION_REQUEST_PLAYER | Handled |
| 0x808B | C -> S | 1 | 56 | GAME_CMSG_CHAR_CREATION_CONFIRM | Handled |
| 0x8090 | C -> S | 37 | 24 | GAME_CMSG_INSTANCE_LOAD_REQUEST_PLAYERS | Handled |
| 0x8091 | C -> S | 37 | 12 | GAME_CMSG_INSTANCE_LOAD_REQUEST_ITEMS | Handled |
| 0x8093 | C -> S | 5 | 8 | GAME_CMSG_INSTANCE_QUERY | Observed |
| 0x80B1 | C -> S | 6 | 24 | GAME_CMSG_PARTY_TRAVEL | Handled |
| 0x80C1 | C -> S | 53 | 12 | GAME_CMSG_TRAVEL_INIT | Observed |

## What the list is for

- `not in our table` - a header the client or server really used that this tree cannot name. Each is a row to
  identify, and the captures that carry it are named in `recordings/index.jsonl`.
- `Unnamed` - a declared row with no name yet; `Known` - named, nothing acts on it; `Handled`/`Emitted` - this
  tree answers or sends it.
- A message being unrelated to a feature is not a reason to leave it out of this table.
