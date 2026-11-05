# Glossary

Plain-language definitions for every word this project leans on. If a document uses a term without
explaining it, it is defined here.

### The programs

**ASURA** - the project. *Automated Server Utility for Remote Authentication.* Not a program: the
whole server stack, made of the components below.

**Socket layer** (`asura-gateway`) - the program that listens on the address a client dials. It accepts
both of a client's connections, answers the file check, reads the four bytes that name the connection
kind, and hands the socket to the auth conversation or relays it to a world. It holds no state.

**auth service** (`asura-auth`) - the login conversation: key exchange, session, character list,
character select. It owns identity, it never runs a map, and since `docs/38-layer-decisions.md` D26 it
is a library the socket layer calls rather than a program with a listener of its own.

**Rules** (`asura-rules`) - the game rules a protocol answer needs, as pure functions over data the
caller has already read: where a character enters the game, for now. No sockets, no database, no
printing (`docs/38`, D25).

**World server** (`asura-world`) - this project's program that runs maps for players, in Rust. The one
in `crates/asura-world` is the whole of it: connection, load block, movement, travel, chat, items. It
is disposable: restarting one loses a session, not the database.

**Control plane** - shorthand for everything that is not simulation: auth, identity, registry,
placement. It spans the auth service and the registry tables, not one process.

**Simulator** - the half of a world server that populates an instance's agent array and advances it:
enemies, gadgets, items, and the characters standing in it. In this tree it is `asura-world`, and today
it populates exactly one agent, the character.

**Registry** - the `world` table: one row per world server with its address, port, flags, population
and last heartbeat. Worlds write their own row; the auth service reads it. Modelled on AzerothCore's
`realmlist`.

**Pool capacity** - `world_pool`: for each world and map, how many seats the world offers (`capacity`)
and how many are currently handed out (`leased`). The seats are counted from the same `capacity` the
world was started with; `docs/01-scope.md` "Instances" says how the maps behind them are built.

**Reservation / lease** - taking a pool slot for one party. It is a single atomic `UPDATE ... WHERE
leased < capacity`, released when the party leaves, when the player disconnects, or when a reaper
decides the owning world is gone.

**Connection pool** - a small set of open database connections reused by a program, instead of one
connection per map instance. This is what replaces GWPS's per-instance SQLite handle.

**Map updater** - the thing that advances a map's simulation. What exists is one task per map, started
by the first join and ended with the instance (`crates/asura-world/src/instance.rs:376-385`). The
bounded pool of threads with map affinity, where a hot map is always handled by the same thread and
idle maps cost nothing, is the AzerothCore structure this project has **not** built; it is the shape
`docs/32` phase 3 and `docs/adr/0006` name for the simulation stage.

**GWPS** - the existing single-process project (`PreSearing`). It does the job of the auth service *and* the
world servers in one program. It stays as the working reference.

### Maps and players

**The game's nouns are the ones to use, and `docs/22-world-model.md` is where they are defined.** The
entries below keep the sense this project has used; where the owner's model says a word means something
else, the owner's sense comes first and the entry says so.

**Account** - the login. It holds the characters and the account's own unlocks. The auth service owns it
(`account`, `session`), and it is never a place and never an agent. See `docs/22-world-model.md`.

**Character** - a body with levels, skills, armour, items and attributes that stands in an instance. The
client's own messages call it **PLAYER** (`GAME_SMSG_PLAYER_CREATE`), and this tree's code calls it
`Player` (`crates/asura-world/src/player.rs`); both are the game's wire, the noun is *character*. A
character is not the login, and it is not the `player_id` a party message carries.

**Agent** - anything inside an instance that is not a prop: enemies, characters, gadgets, items. The
character is an agent. The **agent array** is the per-instance list of them, which the instance's
simulator populates. **An instance is not an agent.**

**Prop** - the half of an instance that is scenery: geometry the client draws and the agent array never
lists.

**Map** (the game's word is **instance**) - a place in the game: Ascalon City, Lakeside County.

**Outpost / town** - an instance everyone shares. All characters in Ascalon City see each other.

**Explorable** - an instance a single party shares privately. Two parties in Lakeside County are in two
separate copies and do not meet.

**Instance** - has two senses, and the game's comes first: (1) a map - the thing a travel moves a
character between, and the thing an unlock bit is set for; (2) this project's older sense, one running
copy of a map. Three solo characters in Lakeside = three copies; the map is one instance.

**District** - a numbered copy of the *same* map, used when a shared map is full. Ascalon City 1, 2,
3... are separate instances of the same outpost.

**Party** - the group of players that shares an explorable. One player counts as a party of one.

**Map instance count** - how many copies of a map exist at once. This is what costs memory.

### Movement

**Travel / hop** - moving a character from one instance to another, as the game does when you walk through
a portal. The client disconnects, is told where to go, and reconnects.

**Portal** - the crossing between two instances, inside one of them. It is a **prop** with a position and
a portal model, read from the instance's own map data, not an agent (`docs/22-world-model.md`).

**Unlock** - one bit per instance in a bitmap that starts all zero, set when the character reaches that
instance. Reaching is what unlocks; the bit never gates the walk. The world map's fast travel is the
functionality the bits allow. Which bitmap, and whether it belongs to the account or the character, is
open (`docs/22-world-model.md`, questions 1 and 2).

**Redirect** - the message that tells the client where to go: an address, a port, a map, and a key.
Until the client receives one, it does not move.

**Token** - a value the auth service hands out for one travel, so that when the client reconnects, the new
connection can be recognised as the one that was expected. A wrong token is refused.

**Allocation** - deciding which world server a travel request goes to, and saying no if none fits.

**Refusal** - the honest answer when there is no room: the client is told nothing about a
destination it cannot go to. It never enters a loading screen.

### Prebuilding

**Prebuild / pre-cache** - build map servers *before* anybody needs them, at startup.

**Pool** - a set of prebuilt instances waiting to be handed out. A party takes one, and returns it
cleared when it leaves.

**The one rule** - a travel request is only ever answered with something already running. Nothing
is constructed while a client is loading. Violating this is what made hops hang in GWPS. One exception
is written into the tree: `asura-world --on-demand` builds a missing map inside the travel, measured at
0-2 ms and accepted by a real client (`crates/asura-world/src/travel.rs:83`,
`docs/03-protocol.md` 5c). It is off unless the flag is given.

### The network

**Handshake** - the opening exchange where the client and server agree a shared secret so the rest
of the conversation is encrypted.

**DH (Diffie-Hellman)** - the specific handshake scheme the client uses, with fixed constants baked
into both the server and the patched client.

**ARC4** - the stream cipher the client uses on everything after the handshake.

**Framing** - how messages are delimited on the wire: a header, a size, then a body.

**No world link** - there is deliberately no control protocol between the auth service and the world
servers. They coordinate through the registry and pool tables. The only protocols left are the
client-facing ones.

**Heartbeat** - a periodic "still here, this full, this busy" message from a world server, so the auth service
can tell a live world from a dead one.

**Directory** - the auth service's in-memory list of which world servers exist, what they host, and how full
they are.

### Machinery

**Thread** - one line of work inside a program. The CPU can only do one thing per core at a time;
threads are the things it switches between.

**Reactor** - one thread that watches many connections at once and handles whichever is ready. Lets
many instances share a few threads instead of one thread each.

**mmap** - reading a file's index straight from disk without loading a private copy into memory,
so many copies of the same data can share it.

**`Gw.dat`** - the game's asset archive, several gigabytes. Our servers read an index of it; the
client streams the rest from ArenaNet's file servers.

**Manifest** - the list of files the client is told to fetch. Serving it ourselves is what would
make the server fully independent of ArenaNet; it is a separate service, not part of the auth service.
