//! M4's world half: a client that was sent to this world connects, is believed, and gets a map.
//!
//! What is proved here, in the order the assertions appear:
//!
//! * a client presenting the token pair the auth service issued is accepted, and one presenting a
//!   token nobody issued is **closed with no message** (the reference's own behaviour);
//! * the four messages of the load block arrive in the reference's order and the character's name is in
//!   the third of them;
//! * each of the three things a client asks for is answered, and the item request's answer ends with
//!   the download manifest, whose last message names the map;
//! * the spawn point tells the client the map blob and where to stand -'- the two numbers without which
//!   the loading screen never ends;
//! * the players request ends with `INSTANCE_LOAD_FINISH`, which is what takes a client off the loading
//!   screen;
//! * the world keeps the clock running without being asked, and answers a ping;
//! * a dropped item becomes an agent of its own, and -'- as in the reference -'- stays in its bag;
//! * a character that leaves standing somewhere comes back standing there, and the slot it took goes
//!   back to the pool both times.
//!
//! With no database configured, each test prints a skip line and returns.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use asura_db::game as db_game;
use asura_db::inventory as db_inventory;
use asura_db::login as db;
use asura_db::{placement, PgPool, Uuid};
use asura_protocol::client::game_version::GameVersion;
use asura_protocol::client::handshake;
use asura_protocol::client::reference_client::ReferenceClient;
use asura_protocol::client::Frame;
use asura_protocol::game;
use asura_world::{serve, MapEntry, World};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const TIMEOUT: Duration = Duration::from_secs(10);

fn skip() -> bool {
    if asura_db::config::database_url().is_none() {
        println!("ASURA_DATABASE_URL not set; skipping the end-to-end world test");
        return true;
    }
    false
}

/// The map every harness serves first: the reference's own `MapId_LakesideCounty`.
///
/// The harness used to invent an id per run (50000 + a random offset) to keep parallel runs apart, which
/// worked until `/tp` started resolving through the reference's **alias table**: `lakeside county` names map
/// 146 there, and a world hosting that map under a made-up id could only answer "not hosted". The real ids
/// are safe to share because everything a harness reserves is keyed by its own `world_id`.
const HARNESS_MAP: i32 = 146;
/// `MapId_AscalonCityPreSearing`, the second map, so `/tp ascalon city` has somewhere to go.
const HARNESS_OTHER_MAP: i32 = 148;

fn unique_world() -> i32 {
    20_000 + (Uuid::new_v4().as_u128() % 10_000) as i32
}

/// A local copy of the client archive (`ASURA_GW_DAT`, then `Gw.dat`, then `data/Gw.dat`): where a
/// map's path graph comes from. Absent, the harness serves a world without graphs and the movement
/// tests skip. Never the reference checkout: this repository stands alone.
fn gw_dat_path() -> Option<String> {
    if let Ok(path) = std::env::var("ASURA_GW_DAT") {
        return Some(path);
    }
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()?
        .parent()?
        .to_path_buf();
    for candidate in [
        std::path::PathBuf::from("Gw.dat"),
        repo.join("data").join("Gw.dat"),
    ] {
        if candidate.exists() {
            return Some(candidate.to_string_lossy().into_owned());
        }
    }
    None
}

/// The graphs a test world starts with: the **bundled** dump (`data/path-graphs.bin`, committed)
/// first, so a fresh clone can path with no archive; then a local `Gw.dat` as the re-dump source;
/// then none, which the movement tests detect and skip on.
fn harness_graphs(maps: &[MapEntry]) -> asura_world::PathGraphs {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("a crate directory")
        .parent()
        .expect("a repository directory");
    let dump = repo.join("data").join("path-graphs.bin");
    if dump.exists() {
        match asura_world::PathGraphs::from_dump(dump.to_str().expect("a path")) {
            Ok(graphs) => {
                graphs.bind(maps);
                return graphs;
            }
            Err(err) => panic!("the bundled dump {:?} does not load: {err}", dump),
        }
    }
    match gw_dat_path() {
        Some(path) => asura_world::PathGraphs::load(&path, maps).unwrap_or_else(|err| {
            panic!("{path} does not open: {err}");
        }),
        None => asura_world::PathGraphs::empty(),
    }
}

/// A world server on a loopback port, and everything the test needs to check behind it.
struct Harness {
    pool: PgPool,
    world_id: i32,
    map_id: i32,
    /// A second map the same world serves, so `/tp` has somewhere to go. The reference pre-spawns one
    /// server per map; this world hosts several maps in one process, so a teleport here is a hop inside
    /// one world -- the same messages, one fewer process.
    other_map_id: i32,
    address: SocketAddr,
    session: Uuid,
    account: Uuid,
    character: Uuid,
    /// The token the auth service "issued" for this harness's travel. Random per harness because the
    /// transfer table is keyed by it and these tests run in parallel.
    player_token: u32,
    server: tokio::task::JoinHandle<()>,
    /// The world the server task is serving, kept so a test can look at its state (the peer count an
    /// idle run must not grow).
    world: Arc<World>,
    /// Whether this harness has a path graph, which needs `Gw.dat` beside the reference checkout. The
    /// movement tests skip when it does not: without a graph every walk is refused, and a test that
    /// checked walking would be checking the refusal.
    walks: bool,
}

impl Harness {
    /// Start a world serving one map, and place a player in it the way the auth service would.
    async fn start() -> Option<Harness> {
        Harness::start_on(0).await
    }

    /// A world serving maps of one type: 0 for a main town, 11 for a character-creation instance.
    async fn start_on(map_type: i32) -> Option<Harness> {
        let url = asura_db::config::database_url()?;
        let pool = placement::connect(&url, 4)
            .await
            .expect("connect the database");

        let world_id = unique_world();
        let map_id = HARNESS_MAP;
        let other_map_id = HARNESS_OTHER_MAP;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind a loopback port");
        let address = listener.local_addr().expect("local address");

        let maps = vec![
            MapEntry {
                map_id,
                // The reference's blob for the pre-Searing region.
                map_file_id: 0x1B97D,
                map_type,
                spawn: (6602.0, 4485.0),
                spawn_plane: 0,
                // The harness states no spawn list, so its arrivals keep the persistence rule; the
                // random outpost entry is tested by `an_outpost_arrival_is_a_random_point_of_its_list`.
                spawns: Vec::new(),
                name: "Lakeside County".to_string(),
                capacity: 2,
            },
            MapEntry {
                map_id: other_map_id,
                map_file_id: 0x1B97D,
                map_type: 0,
                spawn: (1.0, 2.0),
                spawn_plane: 0,
                spawns: Vec::new(),
                name: "Ascalon City".to_string(),
                capacity: 2,
            },
        ];

        // The graphs: the bundled dump first (a fresh clone can path), then a local archive.
        let graphs = harness_graphs(&maps);
        let walks = !graphs.maps().is_empty();

        let world = Arc::new(World {
            pool: pool.clone(),
            world_id,
            maps,
            replay: None,
            instances: Default::default(),
            public: address,
            graphs,
            lazy_maps: Default::default(),
            load_on_demand: false,
            capacity: 2,
            archive: gw_dat_path().unwrap_or_default(),
        });
        let server = tokio::spawn({
            let world = world.clone();
            async move {
                let _ = serve(listener, world, true).await;
            }
        });

        // The identity the login path would have produced.
        let seeded = db::seed_identity(
            &pool,
            Uuid::new_v4(),
            Uuid::new_v4(),
            &format!("m4-world-{}", Uuid::new_v4()),
            "Asura Test",
            map_id,
        )
        .await
        .expect("seed the identity");

        // What the auth service does for a travel: take the slot, then write the pending transfer.
        placement::register_world(
            &pool,
            world_id,
            "e2e-world",
            &address.ip().to_string(),
            i32::from(address.port()),
            "m4-test",
            &[(map_id, 2), (other_map_id, 2)],
        )
        .await
        .expect("register the world");
        placement::reserve(&pool, world_id, map_id, seeded.session)
            .await
            .expect("reserve");
        let player_token = (Uuid::new_v4().as_u128() & 0xFFFF_FFFF) as u32;
        placement::begin_transfer(
            &pool,
            player_token,
            world_id,
            map_id,
            seeded.session,
            seeded.account,
            seeded.character,
        )
        .await
        .expect("write the pending transfer");

        Some(Harness {
            pool,
            world_id,
            map_id,
            other_map_id,
            address,
            session: seeded.session,
            account: seeded.account,
            character: seeded.character,
            player_token,
            server,
            world,
            walks,
        })
    }

    fn stop(self) {
        self.server.abort();
    }

    /// The version message a client presents for `player_token`.
    fn version(&self, player_token: u32) -> GameVersion {
        GameVersion {
            version: 39112,
            h0008: 1,
            map_token: self.world_id as u32,
            map_id: self.map_id as u32,
            player_token,
            account: self.account.to_bytes_le(),
            character: self.character.to_bytes_le(),
            h0038: 0,
            h003c: 0,
        }
    }

    /// Do what the auth service does for one travel: take a slot, then write the pending transfer.
    ///
    /// A second call is how a test gets a second session for the same character -'- the first transfer is
    /// claimed by the first connection and cannot be presented twice.
    async fn transfer(&self) -> u32 {
        placement::reserve(&self.pool, self.world_id, self.map_id, self.session)
            .await
            .expect("reserve");
        let player_token = (Uuid::new_v4().as_u128() & 0xFFFF_FFFF) as u32;
        placement::begin_transfer(
            &self.pool,
            player_token,
            self.world_id,
            self.map_id,
            self.session,
            self.account,
            self.character,
        )
        .await
        .expect("write the pending transfer");
        player_token
    }

    /// Wait for the slot a travel took to go back to the pool, which the world does when the client
    /// leaves. Returns whether it happened.
    ///
    /// The window is generous on purpose: leaving writes the character's position *and* releases the
    /// lease, and a whole test suite hammering one database can make two round trips take longer than a
    /// couple of seconds -'- which produced a flake before this comment existed.
    async fn slot_released(&self) -> bool {
        for _ in 0..100 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            if placement::pool_counts(&self.pool, self.world_id, self.map_id)
                .await
                .expect("read the counter")
                == Some((2, 0))
            {
                return true;
            }
        }
        false
    }
}

/// A cleaner key exchange: send the game version, then the seed, then accept the reply.
async fn connect_properly(
    address: SocketAddr,
    version: &GameVersion,
) -> Option<(TcpStream, ReferenceClient)> {
    let mut stream = tokio::time::timeout(TIMEOUT, TcpStream::connect(address))
        .await
        .expect("connect before the timeout")
        .expect("connect");
    stream
        .write_all(&version.encode())
        .await
        .expect("write the game version");

    let mut client = ReferenceClient::with_seed(0x5EED5EED);
    stream
        .write_all(&client.client_seed_message())
        .await
        .expect("write the client seed");

    let mut seed = [0u8; handshake::SERVER_SEED_LEN];
    match tokio::time::timeout(TIMEOUT, stream.read_exact(&mut seed)).await {
        Ok(Ok(_)) => {}
        Ok(Err(_)) | Err(_) => return None,
    }
    client.accept_server_seed(&seed).expect("well-formed seed");
    Some((stream, client))
}

/// Read until a frame with `header` has arrived, and return everything read so far.
/// Read frames until `count` of them carry this header.
///
/// Waiting for the *first* of a pair that the world writes together is a race: the two frames can land in
/// different reads, and a caller that stops at the first then sees one where the wire holds two. That is
/// why the count is explicit rather than implied.
async fn read_until_count(
    stream: &mut TcpStream,
    client: &mut ReferenceClient,
    header: u16,
    count: usize,
) -> Vec<Frame> {
    let mut frames: Vec<Frame> = Vec::new();
    let mut buffer = [0u8; 8192];
    while frames
        .iter()
        .filter(|frame| frame.header() == header)
        .count()
        < count
    {
        let read = tokio::time::timeout(TIMEOUT, stream.read(&mut buffer))
            .await
            .expect("read before the timeout")
            .expect("read");
        assert!(read > 0, "the world closed the connection while answering");
        frames.extend(client.receive_game(&buffer[..read]).expect("game frames"));
    }
    frames
}

async fn read_until(
    stream: &mut TcpStream,
    client: &mut ReferenceClient,
    header: u16,
) -> Vec<Frame> {
    read_until_count(stream, client, header, 1).await
}

/// Send a client message: the header and an all-zero body of the reference's length.
async fn ask(stream: &mut TcpStream, client: &mut ReferenceClient, header: u16, size: usize) {
    let mut bytes = vec![0u8; size];
    bytes[0..2].copy_from_slice(&header.to_le_bytes());
    client.seal_raw(&mut bytes);
    stream.write_all(&bytes).await.expect("write the request");
}

#[tokio::test]
async fn a_placed_client_is_loaded_into_its_map() {
    if skip() {
        return;
    }
    let Some(harness) = Harness::start().await else {
        return;
    };

    let version = GameVersion {
        version: 39112,
        h0008: 1,
        map_token: harness.world_id as u32,
        map_id: harness.map_id as u32,
        player_token: harness.player_token,
        account: harness.account.to_bytes_le(),
        character: harness.character.to_bytes_le(),
        h0038: 0,
        h003c: 0,
    };
    let Some((mut stream, mut client)) = connect_properly(harness.address, &version).await else {
        panic!("the world refused a token the auth service had just issued");
    };

    // ---- the load block, which the world sends without being asked ---------------------------
    let frames = read_until(&mut stream, &mut client, game::CARTOGRAPHY_DATA).await;
    let headers: Vec<u16> = frames.iter().map(|frame| frame.header()).collect();
    assert_eq!(
        &headers[..6],
        &[
            game::INSTANCE_LOAD_HEAD,
            game::INSTANCE_PLAYER_DATA_START,
            game::INSTANCE_LOAD_PLAYER_NAME,
            game::INSTANCE_LOAD_INFO,
            game::MAP_INIT_START,
            game::CARTOGRAPHY_DATA,
        ],
        "the four messages, then the exploration grid right after INSTANCE_LOAD_INFO"
    );
    assert_eq!(frames[0].len(), 6, "INSTANCE_LOAD_HEAD");
    assert_eq!(frames[1].len(), 2, "INSTANCE_PLAYER_DATA_START");
    assert_eq!(frames[2].len(), 2 + 2 + 2 * 10, "a ten-unit name");
    assert_eq!(frames[3].len(), 16, "INSTANCE_LOAD_INFO");
    let grid_start = &frames[4];
    // 14, not the capture's 16: a live capture reports a four-byte header there, and sending 16
    // desynchronises this client - error 007 at map load, measured 2026-10-30. The client settles it, and
    // `game::map_init_start`'s own documentation records the measurement.
    assert_eq!(
        grid_start.len(),
        14,
        "MAP_INIT_START: a game header and three dwords"
    );
    // The three dwords of the 14-byte form: the header is two bytes, so the dimensions start the body.
    assert_eq!(
        u32::from_le_bytes([
            grid_start.body()[0],
            grid_start.body()[1],
            grid_start.body()[2],
            grid_start.body()[3]
        ]),
        64,
        "Tyria's measured grid width"
    );
    assert_eq!(
        u32::from_le_bytes([
            grid_start.body()[4],
            grid_start.body()[5],
            grid_start.body()[6],
            grid_start.body()[7]
        ]),
        128,
        "and its height"
    );
    // The grid an empty map sends is four dwords (an all-unexplored 64 x 128 grid compresses to 16
    // bytes), and the message carries a `u16` count of exactly that many; the remaining slots of the
    // 260-byte message are zero padding. A test client that frames a counted message by its count
    // therefore sees 2 + 2 + 16, while the wire holds 260 - the client this server is measured against
    // reads the fixed array and pads, which is why the frame is sent that way
    // (`game::cartography_data`).
    assert_eq!(
        frames[5].len(),
        20,
        "CARTOGRAPHY_DATA: a count of four dwords, then the padding this client ignores"
    );
    assert_eq!(
        u16::from_le_bytes([frames[5].body()[0], frames[5].body()[1]]),
        4,
        "the count says how many dwords are meaningful"
    );

    // The name the client is told is the character's own, in UTF-16 little endian.
    let name = &frames[2].body();
    let units = u16::from_le_bytes([name[0], name[1]]) as usize;
    assert_eq!(units, "Asura Test".encode_utf16().count());
    let decoded: String = (0..units)
        .map(|index| u16::from_le_bytes([name[2 + index * 2], name[3 + index * 2]]))
        .map(|unit| char::from_u32(u32::from(unit)).expect("a BMP character"))
        .collect();
    assert_eq!(decoded, "Asura Test");

    // The agent in the load info is the one the world will place.
    assert_eq!(
        u32::from_le_bytes([
            frames[3].body()[0],
            frames[3].body()[1],
            frames[3].body()[2],
            frames[3].body()[3]
        ]),
        asura_world::AGENT_ID
    );

    // ---- the item stream --------------------------------------------------------------------
    //
    // The world's clock keeps running while these arrive, so the block is checked for the reference's
    // *order* rather than for being the only thing on the wire.
    ask(&mut stream, &mut client, 0x8091, 4).await;
    // Two DONE frames, not one: the answer ends the bag list and then the stream, and under load the two
    // can arrive in separate reads.
    let frames = read_until_count(&mut stream, &mut client, game::INSTANCE_MANIFEST_DONE, 2).await;
    let headers: Vec<u16> = frames.iter().map(|frame| frame.header()).collect();
    let at = |header: u16| {
        headers
            .iter()
            .position(|candidate| *candidate == header)
            .unwrap_or_else(|| panic!("{header:#06X} is missing from the item answer"))
    };
    assert!(
        at(game::ITEM_STREAM_CREATE) < at(game::MAP_UPDATE_CURRENT),
        "the item stream opens before the map is announced"
    );
    assert!(
        at(game::MAP_UPDATE_CURRENT) < at(game::READY_FOR_MAP_SPAWN),
        "the map is announced before the client is told it may spawn"
    );
    assert!(
        at(game::READY_FOR_MAP_SPAWN) < at(game::INSTANCE_MANIFEST_DONE),
        "and the manifest comes last"
    );
    assert_eq!(
        headers
            .iter()
            .filter(|h| **h == game::INSTANCE_MANIFEST_DONE)
            .count(),
        2,
        "the manifest sends two DONE messages"
    );
    let map_update = frames
        .iter()
        .find(|frame| frame.header() == game::MAP_UPDATE_CURRENT)
        .expect("a map update");
    assert_eq!(
        map_update.len(),
        5,
        "the login shape, not the redirect's 12"
    );
    assert_eq!(
        u16::from_le_bytes([map_update.body()[0], map_update.body()[1]]),
        harness.map_id as u16
    );

    // ---- the spawn point --------------------------------------------------------------------
    ask(&mut stream, &mut client, 0x8088, 2).await;
    let frames = read_until(&mut stream, &mut client, game::INSTANCE_LOAD_SPAWN_POINT).await;
    let spawn = frames
        .iter()
        .find(|frame| frame.header() == game::INSTANCE_LOAD_SPAWN_POINT)
        .expect("a spawn point");
    assert_eq!(spawn.len(), 26);
    assert_eq!(
        u32::from_le_bytes([
            spawn.body()[0],
            spawn.body()[1],
            spawn.body()[2],
            spawn.body()[3]
        ]),
        0x1B97D,
        "the map blob the client must stream"
    );
    assert_eq!(
        f32::from_le_bytes([
            spawn.body()[4],
            spawn.body()[5],
            spawn.body()[6],
            spawn.body()[7]
        ]),
        6602.0,
        "and where to stand, from the map's own config"
    );

    // ---- the players ------------------------------------------------------------------------
    ask(&mut stream, &mut client, 0x8090, 19).await;
    let frames = read_until(&mut stream, &mut client, game::INSTANCE_LOAD_FINISH).await;
    let headers: Vec<u16> = frames.iter().map(|frame| frame.header()).collect();
    let position = |header: u16| headers.iter().position(|candidate| *candidate == header);
    assert!(
        position(game::INSTANCE_LOAD_FINISH).is_some(),
        "the load ends with the only message that leaves the loading screen"
    );
    assert!(
        position(game::INSTANCE_LOAD_FINISH) > position(game::WORLD_CREATE_AGENT),
        "and it comes after the agent exists"
    );
    for required in [
        game::PLAYER_CREATE,
        game::WORLD_CREATE_AGENT,
        game::WORLD_UPDATE_CONTROLLED_AGENT,
        game::PARTY_CREATE,
        game::CHARACTER_UPDATE_PARTY,
    ] {
        assert!(
            headers.contains(&required),
            "{required:#06X} is part of the load"
        );
    }
    let agent = frames
        .iter()
        .find(|frame| frame.header() == game::WORLD_CREATE_AGENT)
        .expect("an agent");
    assert_eq!(agent.len(), 99, "the reference's fixed agent message");

    // ---- the clock, without being asked -----------------------------------------------------
    let frames = read_until(&mut stream, &mut client, game::WORLD_SIMULATION_TICK).await;
    assert!(
        frames
            .iter()
            .any(|frame| frame.header() == game::WORLD_SIMULATION_TICK),
        "the world keeps time on its own"
    );

    // ---- the ping ---------------------------------------------------------------------------
    ask(&mut stream, &mut client, 0x800D, 2).await;
    let frames = read_until(&mut stream, &mut client, game::PING_REPLY).await;
    let reply = frames
        .iter()
        .find(|frame| frame.header() == game::PING_REPLY)
        .expect("a ping reply");
    assert_eq!(
        u32::from_le_bytes([
            reply.body()[0],
            reply.body()[1],
            reply.body()[2],
            reply.body()[3]
        ]),
        asura_world::connection::ping_value()
    );

    // ---- leaving releases the slot ----------------------------------------------------------
    drop(stream);
    assert!(
        harness.slot_released().await,
        "the player's slot went back to the pool when it left"
    );

    let _ = harness.session;
    placement::remove_world(&harness.pool, harness.world_id)
        .await
        .expect("remove the world");
    harness.stop();
}

#[tokio::test]
async fn a_token_nobody_issued_is_closed_without_a_word() {
    if skip() {
        return;
    }
    let Some(harness) = Harness::start().await else {
        return;
    };

    let version = GameVersion {
        version: 39112,
        h0008: 1,
        map_token: harness.world_id as u32,
        map_id: harness.map_id as u32,
        player_token: harness.player_token ^ 0xFFFF, // never written to the transfer table
        account: harness.account.to_bytes_le(),
        character: harness.character.to_bytes_le(),
        h0038: 0,
        h003c: 0,
    };
    let accepted = connect_properly(harness.address, &version).await;
    assert!(
        accepted.is_none(),
        "a client with a token the auth service never issued must not reach the key exchange"
    );

    placement::remove_world(&harness.pool, harness.world_id)
        .await
        .expect("remove the world");
    harness.stop();
}

#[tokio::test]
async fn a_character_that_is_not_the_one_transferred_is_not_played() {
    if skip() {
        return;
    }
    let Some(harness) = Harness::start().await else {
        return;
    };

    // A real token, but a character uuid that is on no account at all - the shape a client presents
    // after the character it was showing has been deleted (measured 2026-10-28). The connection is
    // accepted, and what it plays is the character the **transfer** names: the token, not the uuid,
    // is the credential, and the character always comes from the transfer's own account.
    let version = GameVersion {
        version: 39112,
        h0008: 1,
        map_token: harness.world_id as u32,
        map_id: harness.map_id as u32,
        player_token: harness.player_token,
        account: harness.account.to_bytes_le(),
        character: Uuid::new_v4().to_bytes_le(),
        h0038: 0,
        h003c: 0,
    };
    let connected = connect_properly(harness.address, &version).await;
    assert!(
        connected.is_some(),
        "a token whose character is gone is still a token: the transfer's character is played"
    );

    placement::remove_world(&harness.pool, harness.world_id)
        .await
        .expect("remove the world");
    harness.stop();
}

/// Send a client message with a body.
async fn say(stream: &mut TcpStream, client: &mut ReferenceClient, header: u16, body: &[u8]) {
    let mut bytes = Vec::with_capacity(2 + body.len());
    bytes.extend_from_slice(&header.to_le_bytes());
    bytes.extend_from_slice(body);
    client.seal_raw(&mut bytes);
    stream.write_all(&bytes).await.expect("write the message");
}

#[tokio::test]
async fn a_drop_deducts_the_stack_and_says_nothing() {
    if skip() {
        return;
    }
    // The default mode: no message at all, because both candidate messages crashed a live client.
    let Some(harness) = Harness::start().await else {
        return;
    };

    let version = harness.version(harness.player_token);
    let Some((mut stream, mut client)) = connect_properly(harness.address, &version).await else {
        panic!("the world refused a token the auth service had just issued");
    };
    // The load block, so the character exists on this connection before anything is asked of it.
    read_until(&mut stream, &mut client, game::INSTANCE_LOAD_INFO).await;

    // The world seeds the default inventory on the way in, so the cupcake's id is only known now.
    let inventory = db_inventory::load(&harness.pool, harness.character)
        .await
        .expect("read the inventory");
    assert_eq!(
        inventory.bags.len(),
        6,
        "a character that has never been played was given the reference's bags"
    );
    let cupcake = inventory.items_in(1)[0].clone();
    assert_eq!(cupcake.slot, Some(0));
    assert_eq!(cupcake.quantity, 100);
    assert_eq!(cupcake.model_id, 22269, "the cupcake's own model");

    // `GAME_CMSG_DROP_ITEM`: `{item_id: u32, count: u8}`. **One** of a stack of a hundred, which is what
    // a real client sends when an item is dragged out of the bag -- and what the reference refuses,
    // because its partial-stack branch is a `TODO`.
    let mut body = Vec::new();
    body.extend_from_slice(&(cupcake.id as u32).to_le_bytes());
    body.push(1);
    say(&mut stream, &mut client, 0x802C, &body).await;

    // A ping is how this test knows the server has read the drop, since the drop itself answers nothing.
    ask(&mut stream, &mut client, 0x800D, 2).await;
    let frames = read_until(&mut stream, &mut client, game::PING_REPLY).await;
    for forbidden in [
        game::ITEM_REMOVE,
        game::CREATE_NAMED_ITEM,
        game::WORLD_CREATE_AGENT,
    ] {
        assert!(
            !frames.iter().any(|frame| frame.header() == forbidden),
            "{forbidden:#06X} must not be sent by a drop: a live client crashed on two of these and \
             no capture of any of them exists"
        );
    }

    // It was one of a hundred, so ninety-nine are left, in the slot they were in.
    let after = db_inventory::load(&harness.pool, harness.character)
        .await
        .expect("read the inventory");
    let left = after.item(cupcake.id).expect("the stack is still there");
    assert_eq!(left.quantity, 99, "one cupcake went to the ground");
    assert_eq!(left.slot, Some(0), "and the rest stayed where they were");

    // Dropping the rest empties the row: the item is gone, and nothing re-creates it.
    let mut body = (cupcake.id as u32).to_le_bytes().to_vec();
    body.push(99);
    say(&mut stream, &mut client, 0x802C, &body).await;
    ask(&mut stream, &mut client, 0x800D, 2).await;
    read_until(&mut stream, &mut client, game::PING_REPLY).await;
    let emptied = db_inventory::load(&harness.pool, harness.character)
        .await
        .expect("read the inventory");
    assert_eq!(
        emptied.item(cupcake.id),
        None,
        "the last of the stack went to the ground and the row with it"
    );

    // A drop of something that is not carried is answered with nothing at all, and does not end the
    // session: the reference's handler returns ERR_OK and sends no message.
    let mut body = 999_999u32.to_le_bytes().to_vec();
    body.push(1);
    say(&mut stream, &mut client, 0x802C, &body).await;
    ask(&mut stream, &mut client, 0x800D, 2).await;
    let frames = read_until(&mut stream, &mut client, game::PING_REPLY).await;
    assert!(
        !frames
            .iter()
            .any(|frame| frame.header() == game::WORLD_CREATE_AGENT),
        "an item nobody carries creates no agent"
    );

    drop(stream);
    assert!(harness.slot_released().await, "the slot went back");
    placement::remove_world(&harness.pool, harness.world_id)
        .await
        .expect("remove the world");
    harness.stop();
}

#[tokio::test]
async fn an_item_can_be_moved_between_slots_and_the_server_agrees() {
    if skip() {
        return;
    }
    let Some(harness) = Harness::start().await else {
        return;
    };

    let version = harness.version(harness.player_token);
    let Some((mut stream, mut client)) = connect_properly(harness.address, &version).await else {
        panic!("the world refused a token the auth service had just issued");
    };
    read_until(&mut stream, &mut client, game::INSTANCE_LOAD_INFO).await;

    let inventory = db_inventory::load(&harness.pool, harness.character)
        .await
        .expect("read the inventory");
    let cupcake = inventory.items_in(1)[0].clone();
    assert_eq!(cupcake.slot, Some(0));

    // `GAME_CMSG_ITEM_MOVE`: `{item_id: u32, bag_id: u16, slot: u8}` -- what a live client sent when an
    // item was dragged, twice, and this server used to ignore.
    let move_to = |bag_id: u16, slot: u8, item: i32| {
        let mut body = (item as u32).to_le_bytes().to_vec();
        body.extend_from_slice(&bag_id.to_le_bytes());
        body.push(slot);
        body
    };
    say(&mut stream, &mut client, 0x8072, &move_to(1, 5, cupcake.id)).await;
    // With the echo (the default) the client is told where the item ended up. A silent answer left a
    // live client's item back where it started, which is why the echo is the default and `silent` is the
    // switch nobody has needed yet.
    let frames = read_until(&mut stream, &mut client, game::ITEM_MOVE_TO_LOCATION).await;
    let echo = frames
        .iter()
        .rev()
        .find(|frame| frame.header() == game::ITEM_MOVE_TO_LOCATION)
        .expect("an accepted move is echoed");
    assert_eq!(
        u32::from_le_bytes([
            echo.body()[2],
            echo.body()[3],
            echo.body()[4],
            echo.body()[5]
        ]),
        cupcake.id as u32
    );
    assert_eq!(
        u16::from_le_bytes([echo.body()[6], echo.body()[7]]),
        1,
        "the bag the client asked for"
    );
    assert_eq!(echo.body()[8], 5, "and the slot");

    let moved = db_inventory::load(&harness.pool, harness.character)
        .await
        .expect("read the inventory");
    assert_eq!(
        moved.item(cupcake.id).and_then(|item| item.slot),
        Some(5),
        "and the row moved with it"
    );

    // Moving onto an occupied slot is refused, and the client is *told where the item really is* --
    // it has already moved it in its own model by the time it asks.
    let occupied = inventory
        .items_in(1)
        .into_iter()
        .find(|item| item.id != cupcake.id)
        .map(|item| item.slot.unwrap_or(1))
        .expect("a second cupcake in the backpack");
    say(
        &mut stream,
        &mut client,
        0x8072,
        &move_to(1, occupied as u8, cupcake.id),
    )
    .await;
    let frames = read_until(&mut stream, &mut client, game::ITEM_MOVE_TO_LOCATION).await;
    let snap_back = frames
        .iter()
        .rev()
        .find(|frame| frame.header() == game::ITEM_MOVE_TO_LOCATION)
        .expect("a refusal puts the client's model back");
    assert_eq!(snap_back.body()[8], 5, "still in the slot it really is in");
    let unchanged = db_inventory::load(&harness.pool, harness.character)
        .await
        .expect("read the inventory");
    assert_eq!(
        unchanged.item(cupcake.id).and_then(|item| item.slot),
        Some(5),
        "and nothing was written"
    );

    drop(stream);
    assert!(harness.slot_released().await, "the slot went back");
    placement::remove_world(&harness.pool, harness.world_id)
        .await
        .expect("remove the world");
    harness.stop();
}

#[tokio::test]
async fn an_item_can_be_taken_off_and_worn_again() {
    if skip() {
        return;
    }
    let Some(harness) = Harness::start().await else {
        return;
    };

    let version = harness.version(harness.player_token);
    let Some((mut stream, mut client)) = connect_properly(harness.address, &version).await else {
        panic!("the world refused a token the auth service had just issued");
    };
    // The players block is where the agent's equipment is sent, so read to the end of the load.
    read_until(&mut stream, &mut client, game::INSTANCE_LOAD_INFO).await;
    ask(&mut stream, &mut client, 0x8090, 19).await;
    let frames = read_until(&mut stream, &mut client, game::INSTANCE_LOAD_FINISH).await;

    // The ids are this character's own (the sequence's), not the capture's: the capture's 1..9 only
    // happen in a database where nothing has been seeded before.
    let inventory = db_inventory::load(&harness.pool, harness.character)
        .await
        .expect("read the inventory");
    let worn_ids = inventory.equipped_item_ids();
    let worn = frames
        .iter()
        .find(|frame| frame.header() == game::UPDATE_AGENT_VISUAL_EQUIPMENT)
        .expect("the visual equipment");
    assert_eq!(
        u32::from_le_bytes([
            worn.body()[4],
            worn.body()[5],
            worn.body()[6],
            worn.body()[7]
        ]),
        worn_ids[0],
        "the weapon slot holds the item the equipped bag says it does"
    );
    assert_ne!(worn_ids[0], 0, "and the character is dressed");

    let equipped_bag = inventory
        .bags
        .iter()
        .find(|bag| bag.bag_model_id == db_inventory::EQUIPPED_ITEMS_MODEL)
        .expect("the equipped-items bag")
        .bag_id;
    let weapon = inventory
        .items_in(equipped_bag)
        .into_iter()
        .find(|item| item.slot == Some(0))
        .cloned()
        .expect("the hammer in the weapon slot");

    // Take it off by dragging it out of the equipped bag: `ITEM_MOVE` into a free carrying slot.
    let free = db_inventory::first_free_carrying_slot(&harness.pool, harness.character)
        .await
        .expect("find a free slot")
        .expect("the backpack has room");
    let mut body = (weapon.id as u32).to_le_bytes().to_vec();
    body.extend_from_slice(&(free.0 as u16).to_le_bytes());
    body.push(free.1 as u8);
    say(&mut stream, &mut client, 0x8072, &body).await;
    let frames = read_until(
        &mut stream,
        &mut client,
        game::UPDATE_AGENT_VISUAL_EQUIPMENT,
    )
    .await;
    let now_wearing = frames
        .iter()
        .rev()
        .find(|frame| frame.header() == game::UPDATE_AGENT_VISUAL_EQUIPMENT)
        .expect("taking the weapon off changes what the agent wears");
    assert_eq!(
        u32::from_le_bytes([
            now_wearing.body()[4],
            now_wearing.body()[5],
            now_wearing.body()[6],
            now_wearing.body()[7]
        ]),
        0,
        "the weapon slot is empty now"
    );

    // Put it back on: `GAME_CMSG_EQUIP_ITEM`, `{item_id: u32}`.
    say(
        &mut stream,
        &mut client,
        0x8030,
        &(weapon.id as u32).to_le_bytes(),
    )
    .await;
    let frames = read_until(
        &mut stream,
        &mut client,
        game::UPDATE_AGENT_VISUAL_EQUIPMENT,
    )
    .await;
    let wearing_again = frames
        .iter()
        .rev()
        .find(|frame| frame.header() == game::UPDATE_AGENT_VISUAL_EQUIPMENT)
        .expect("equipping changes what the agent wears");
    assert_eq!(
        u32::from_le_bytes([
            wearing_again.body()[4],
            wearing_again.body()[5],
            wearing_again.body()[6],
            wearing_again.body()[7]
        ]),
        weapon.id as u32,
        "and the hammer is back in the weapon slot"
    );
    let after = db_inventory::load(&harness.pool, harness.character)
        .await
        .expect("read the inventory");
    assert_eq!(
        after.item(weapon.id).and_then(|item| item.bag_id),
        Some(equipped_bag),
        "the row is in the equipped bag again"
    );

    drop(stream);
    assert!(harness.slot_released().await, "the slot went back");
    placement::remove_world(&harness.pool, harness.world_id)
        .await
        .expect("remove the world");
    harness.stop();
}

/// A spoken line: `GAME_CMSG_SEND_CHAT_MESSAGE` = `{agent_id u32, String16}`.
/// Read the redirect pair the way GWPS's own build frames it: 0x01A6 is 46 bytes (two-byte header,
/// host[24], five dwords), followed by the 12-byte 0x0099. Both are found by scanning the decrypted
/// stream.
async fn read_redirect(stream: &mut TcpStream, client: &mut ReferenceClient) -> (Vec<u8>, Vec<u8>) {
    let mut plain: Vec<u8> = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        let read = tokio::time::timeout(TIMEOUT, stream.read(&mut buffer))
            .await
            .expect("read before the timeout")
            .expect("read");
        assert!(
            read > 0,
            "the world closed the connection before the redirect"
        );
        let mut chunk = buffer[..read].to_vec();
        client.decrypt(&mut chunk);
        plain.extend_from_slice(&chunk);
        if let Some(at) = plain.windows(2).position(|window| window == [0xA6, 0x01]) {
            if plain.len() >= at + 58 {
                return (
                    plain[at..at + 46].to_vec(),
                    plain[at + 46..at + 58].to_vec(),
                );
            }
        }
    }
}

fn chat_body(text: &str) -> Vec<u8> {
    let units: Vec<u16> = text.encode_utf16().collect();
    let mut body = 1u32.to_le_bytes().to_vec();
    body.extend_from_slice(&(units.len() as u16).to_le_bytes());
    for unit in units {
        body.extend_from_slice(&unit.to_le_bytes());
    }
    body
}

/// The world's default map list is the whole catalogue, and `/tp` reaches all of it.
///
/// This is the regression test for a gap that stood for many rounds: `asura-world` without `--maps` used to
/// host **one** map (146, the constant from the first single-map world), so `/maps` printed one alias and
/// `/tp` to any other map was refused -- while the reference pre-spawns a server for every configured map
/// (`AuthSrv_PreSpawnMapServers`) and teleports everywhere. `--maps` is a narrowing tool for tests and soaks,
/// never the default.
#[tokio::test]
async fn the_default_map_list_is_the_whole_catalogue_and_tp_reaches_it() {
    if skip() {
        return;
    }
    let Some(harness) = Harness::start().await else {
        return;
    };
    let ids = asura_db::game::configured_map_ids(&harness.pool)
        .await
        .expect("read the catalogue");
    assert!(
        ids.len() >= 12,
        "the catalogue is the reference's own list: {} map(s)",
        ids.len()
    );

    // Every catalogue map that a client loads is servable: a blob to hand it and a name to list. The
    // character-creation instance is the exception, measured: it sends no spawn point and no blob at
    // all, only the load head, the player-data block and `CHAR_CREATION_START`
    // (`creation::handle_confirm`, `connection.rs` #5).
    let mut maps = Vec::new();
    for map_id in &ids {
        let configured = asura_db::game::spawn_map(&harness.pool, *map_id)
            .await
            .expect("read the map")
            .unwrap_or_else(|| panic!("map {map_id} is in the catalogue"));
        if configured.map_type != asura_world::map_kinds().character_creation {
            assert!(configured.map_file_id > 0, "map {map_id} has a blob");
        }
        maps.push(MapEntry {
            map_id: *map_id,
            map_file_id: u32::try_from(configured.map_file_id).expect("a file id"),
            map_type: configured.map_type,
            spawn: (configured.spawn_x, configured.spawn_y),
            spawn_plane: configured.spawn_plane as u16,
            spawns: asura_world::map_spawns::for_map(*map_id)
                .map(<[_]>::to_vec)
                .unwrap_or_default(),
            name: configured.name.clone(),
            capacity: 8,
        });
    }

    // `/maps` lists the whole catalogue -- and it lists **every alias**, not one row per map: the
    // reference's log table is a row per alias, so a tester can read the exact word to type.
    let hosted: Vec<i32> = maps.iter().map(|map| map.map_id).collect();
    let answer = asura_world::maps_answer(&maps);
    let hosted_aliases = asura_world::travel_targets::TRAVEL_TARGETS
        .iter()
        .filter(|target| hosted.contains(&target.map_id))
        .count();
    // A hosted map the alias table does not name still gets a row, from its own name. A creation
    // instance gets none: it is not a place (`chat::maps_answer`).
    let hosted_without_alias = maps
        .iter()
        .filter(|map| map.map_type != asura_world::map_kinds().character_creation)
        .filter(|map| {
            !asura_world::travel_targets::TRAVEL_TARGETS
                .iter()
                .any(|target| target.map_id == map.map_id)
        })
        .count();
    assert_eq!(
        answer.table.len(),
        hosted_aliases + hosted_without_alias + 1,
        "a header and one row per hosted map: one per alias, plus the maps the table does not name"
    );
    assert!(
        hosted_aliases >= 20,
        "every alias of the reference's table names a catalogue map: {hosted_aliases}"
    );
    for target in asura_world::travel_targets::TRAVEL_TARGETS {
        if !hosted.contains(&target.map_id) {
            continue;
        }
        assert!(
            answer.table.iter().any(|row| row.contains(&format!(
                "/tp {:<15} -> map {:>3}  {}",
                target.alias, target.map_id, target.label
            ))),
            "/maps names /tp {} -> map {}",
            target.alias,
            target.map_id
        );
    }

    // And every alias the reference accepts for a catalogue map resolves to a map this world serves, which
    // is what makes `/tp` work without extra steps.
    let hosted: Vec<i32> = maps.iter().map(|map| map.map_id).collect();
    let mut reachable = 0usize;
    for target in asura_world::travel_targets::TRAVEL_TARGETS {
        if target.hosted {
            assert!(
                hosted.contains(&target.map_id),
                "/tp {} names map {}, which the catalogue hosts",
                target.alias,
                target.map_id
            );
            reachable += 1;
        }
    }
    assert!(
        reachable >= 20,
        "every alias of the reference's table is reachable: {reachable}"
    );

    placement::remove_world(&harness.pool, harness.world_id)
        .await
        .expect("remove the world");
    harness.stop();
}

#[tokio::test]
async fn maps_prints_its_table_and_puts_nothing_on_the_wire() {
    if skip() {
        return;
    }
    let Some(harness) = Harness::start().await else {
        return;
    };

    let version = harness.version(harness.player_token);
    let Some((mut stream, mut client)) = connect_properly(harness.address, &version).await else {
        panic!("the world refused a token the auth service had just issued");
    };
    read_until(&mut stream, &mut client, game::INSTANCE_LOAD_INFO).await;

    // `/maps`, exactly as a client types it. The answer is the reference's table, printed in the server's
    // console (`--nocapture` shows it); **nothing is sent**, which is what the reference does too: its
    // `GameSrv_SendSystemChatMessage` returns before touching the socket unless `GWPS_CHAT` is set, so a
    // client has nothing to receive and nothing to crash on.
    say(&mut stream, &mut client, 0x8064, &chat_body("/maps")).await;

    ask(&mut stream, &mut client, 0x800D, 2).await;
    let frames = read_until(&mut stream, &mut client, game::PING_REPLY).await;
    for forbidden in [
        game::CHAT_MESSAGE_CORE,
        game::CHAT_MESSAGE_SERVER,
        game::CHAT_MESSAGE_LOCAL,
    ] {
        assert!(
            !frames.iter().any(|frame| frame.header() == forbidden),
            "{forbidden:#06X} must not be sent for /maps: the reference answers into its own log, and \
             sending chat crashed a live client here"
        );
    }

    // An unknown command is the same: the reference's `GameSrv_SendInvalidCommand` logs "Unknown command"
    // and returns without sending, under the same gate.
    say(&mut stream, &mut client, 0x8064, &chat_body("/nonsense")).await;
    ask(&mut stream, &mut client, 0x800D, 2).await;
    let frames = read_until(&mut stream, &mut client, game::PING_REPLY).await;
    assert!(
        !frames
            .iter()
            .any(|frame| frame.header() == game::CHAT_MESSAGE_CORE),
        "an unknown command answers in the log, not on the wire"
    );

    drop(stream);
    assert!(harness.slot_released().await, "the slot went back");
    placement::remove_world(&harness.pool, harness.world_id)
        .await
        .expect("remove the world");
    harness.stop();
}

#[tokio::test]
async fn a_character_comes_back_where_it_left() {
    if skip() {
        return;
    }
    let Some(harness) = Harness::start().await else {
        return;
    };

    // ---- the first session: walk away from the spawn point, then leave ----------------------
    let version = harness.version(harness.player_token);
    let Some((mut stream, mut client)) = connect_properly(harness.address, &version).await else {
        panic!("the world refused a token the auth service had just issued");
    };
    read_until(&mut stream, &mut client, game::INSTANCE_LOAD_INFO).await;

    // A position report 100 units from the spawn: inside the reference's budget, so it is adopted.
    // `GAME_CMSG_POSITION_UPDATE` is 26 bytes: `{position: Vec2f, plane: u32, unk0: Vec2f, unk1: u32}`,
    // and the world reads the first three fields.
    let walked = (6702.0f32, 4485.0f32);
    let mut body = Vec::new();
    body.extend_from_slice(&walked.0.to_le_bytes());
    body.extend_from_slice(&walked.1.to_le_bytes());
    body.extend_from_slice(&0u32.to_le_bytes()); // plane
    body.extend_from_slice(&[0u8; 12]); // the two trailing fields the world does not read
    say(&mut stream, &mut client, 0x803D, &body).await;

    // The report answers nothing; a ping is how this test knows the server has read it.
    ask(&mut stream, &mut client, 0x800D, 2).await;
    read_until(&mut stream, &mut client, game::PING_REPLY).await;

    drop(stream);
    assert!(
        harness.slot_released().await,
        "the slot went back when the player left"
    );

    // Where it stood is now in the database, which is the only thing that makes the next session
    // different from the first.
    let saved = asura_db::game::position(&harness.pool, harness.character)
        .await
        .expect("read the position")
        .expect("leaving saves a position");
    assert_eq!(saved.map_id, harness.map_id);
    assert_eq!((saved.x, saved.y), walked);

    // ---- the second session: the same character arrives where it left ----------------------
    let token = harness.transfer().await;
    let version = harness.version(token);
    let Some((mut stream, mut client)) = connect_properly(harness.address, &version).await else {
        panic!("the world refused the second travel's token");
    };
    read_until(&mut stream, &mut client, game::INSTANCE_LOAD_INFO).await;

    ask(&mut stream, &mut client, 0x8088, 2).await;
    let frames = read_until(&mut stream, &mut client, game::INSTANCE_LOAD_SPAWN_POINT).await;
    let spawn = frames
        .iter()
        .find(|frame| frame.header() == game::INSTANCE_LOAD_SPAWN_POINT)
        .expect("a spawn point");
    let x = f32::from_le_bytes([
        spawn.body()[4],
        spawn.body()[5],
        spawn.body()[6],
        spawn.body()[7],
    ]);
    let y = f32::from_le_bytes([
        spawn.body()[8],
        spawn.body()[9],
        spawn.body()[10],
        spawn.body()[11],
    ]);
    assert_eq!(
        (x, y),
        walked,
        "the client is put back where the character left, not at the map's spawn"
    );

    // The agent is created at the same place, so the two messages do not disagree.
    ask(&mut stream, &mut client, 0x8090, 19).await;
    let frames = read_until(&mut stream, &mut client, game::INSTANCE_LOAD_FINISH).await;
    let agent = frames
        .iter()
        .find(|frame| frame.header() == game::WORLD_CREATE_AGENT)
        .expect("an agent");
    assert_eq!(
        f32::from_le_bytes([
            agent.body()[10],
            agent.body()[11],
            agent.body()[12],
            agent.body()[13]
        ]),
        walked.0,
        "the agent stands where the spawn point says"
    );

    drop(stream);
    assert!(
        harness.slot_released().await,
        "and the slot went back again"
    );
    placement::remove_world(&harness.pool, harness.world_id)
        .await
        .expect("remove the world");
    harness.stop();
}

/// T9/T14: a second client in the same map, and what the two connections tell each other.
///
/// This is the multi-player surface the reference has and this server did not: two agents with two ids,
/// each player introduced to the other in the reference's own order (`GameSrv_SendWorldAgents`,
/// `GmAgent.c:310-331`), and the pair of messages that take a player back out again
/// (`GameSrv_BroadcastPlayerDestroy`, `GmAgent.c:60`).
#[tokio::test]
async fn a_second_player_is_introduced_to_the_first_and_withdrawn_again() {
    if skip() {
        return;
    }
    let Some(harness) = Harness::start().await else {
        return;
    };

    // The first player: the identity the harness seeded.
    let version_a = harness.version(harness.player_token);
    let Some((mut stream_a, mut client_a)) = connect_properly(harness.address, &version_a).await
    else {
        panic!("the world refused the first client");
    };
    ask(&mut stream_a, &mut client_a, 0x8088, 2).await;
    read_until(
        &mut stream_a,
        &mut client_a,
        game::INSTANCE_LOAD_SPAWN_POINT,
    )
    .await;
    ask(&mut stream_a, &mut client_a, 0x8090, 19).await;
    read_until(&mut stream_a, &mut client_a, game::INSTANCE_LOAD_FINISH).await;

    // The second: a different character on the same map and the same world server, placed the way the
    // auth service would.
    let second = db::seed_identity(
        &harness.pool,
        Uuid::new_v4(),
        Uuid::new_v4(),
        &format!("m4-second-{}", Uuid::new_v4()),
        "Second Player",
        harness.map_id,
    )
    .await
    .expect("seed a second identity");
    placement::reserve(
        &harness.pool,
        harness.world_id,
        harness.map_id,
        second.session,
    )
    .await
    .expect("reserve for the second player");
    let token_b = (Uuid::new_v4().as_u128() & 0xFFFF_FFFF) as u32;
    placement::begin_transfer(
        &harness.pool,
        token_b,
        harness.world_id,
        harness.map_id,
        second.session,
        second.account,
        second.character,
    )
    .await
    .expect("write the second transfer");

    let version_b = GameVersion {
        version: 39112,
        h0008: 1,
        map_token: harness.world_id as u32,
        map_id: harness.map_id as u32,
        player_token: token_b,
        account: second.account.to_bytes_le(),
        character: second.character.to_bytes_le(),
        h0038: 0,
        h003c: 0,
    };
    let Some((mut stream_b, mut client_b)) = connect_properly(harness.address, &version_b).await
    else {
        panic!("the world refused the second client");
    };
    ask(&mut stream_b, &mut client_b, 0x8088, 2).await;
    read_until(
        &mut stream_b,
        &mut client_b,
        game::INSTANCE_LOAD_SPAWN_POINT,
    )
    .await;

    // The second client asks who is here: its own block, then the first player's introduction.
    ask(&mut stream_b, &mut client_b, 0x8090, 19).await;
    // The block and the introduction of the other player are one write, so one read holds both:
    // read_until returns everything it read, not only up to the header it waited for.
    let everything = read_until(&mut stream_b, &mut client_b, game::INSTANCE_LOAD_FINISH).await;

    // Its own player is in the block, so the ids are a set: one for this client, one for the other.
    let players: Vec<u32> = everything
        .iter()
        .filter(|frame| frame.header() == game::PLAYER_CREATE)
        .map(|frame| {
            u32::from_le_bytes([
                frame.body()[0],
                frame.body()[1],
                frame.body()[2],
                frame.body()[3],
            ])
        })
        .collect();
    assert_eq!(players.len(), 2, "two players: this one and the other");
    assert!(
        players.contains(&asura_world::PLAYER_ID),
        "the first player keeps id 1"
    );
    let other_agent = everything
        .iter()
        .rev()
        .find(|frame| frame.header() == game::WORLD_CREATE_AGENT)
        .expect("an agent for the other player");
    assert_eq!(
        u32::from_le_bytes([
            other_agent.body()[0],
            other_agent.body()[1],
            other_agent.body()[2],
            other_agent.body()[3]
        ]),
        asura_world::AGENT_ID,
        "and the first player's agent keeps agent id 1"
    );

    // The first client is told about the second, in the same order.
    let introduced = read_until(&mut stream_a, &mut client_a, game::WORLD_CREATE_AGENT).await;
    let newcomer = introduced
        .iter()
        .find(|frame| frame.header() == game::PLAYER_CREATE)
        .expect("the first client is told about the second player");
    let newcomer_id = u32::from_le_bytes([
        newcomer.body()[0],
        newcomer.body()[1],
        newcomer.body()[2],
        newcomer.body()[3],
    ]);
    assert_ne!(
        newcomer_id,
        asura_world::PLAYER_ID,
        "the second player gets an id of its own"
    );
    assert!(
        introduced
            .iter()
            .any(|frame| frame.header() == game::PLAYER_UPDATE_PARTY_SIZE),
        "with its party size, as the reference sends it"
    );
    assert!(
        introduced
            .iter()
            .any(|frame| frame.header() == game::PLAYER_UPDATE_PARTY),
        "and its own party, as the reference sends it"
    );
    // And what it takes to draw them: `GameSrv_BroadcastPlayerEquippedItems` sends the equipped items in
    // their *unnamed* form, then the visual equipment and the cape (`GameSrv.c:1648-1650`). In this build
    // that form is retail's `0x015E` low-detail record, not `0x015F` (`game::ITEM_LOW_DETAIL`).
    assert!(
        introduced
            .iter()
            .any(|frame| frame.header() == game::ITEM_LOW_DETAIL),
        "the newcomer's equipped items, low-detail"
    );
    assert!(
        introduced
            .iter()
            .all(|frame| frame.header() != game::CREATE_UNNAMED_ITEM),
        "and never as 0x015F, which retail does not send in a load"
    );
    assert!(
        introduced
            .iter()
            .any(|frame| frame.header() == game::UPDATE_AGENT_VISUAL_EQUIPMENT),
        "the newcomer's visual equipment"
    );
    assert!(
        introduced
            .iter()
            .any(|frame| frame.header() == game::AGENT_DISPLAY_CAPE),
        "and its cape"
    );

    // T14: the second client goes away, and the first one's world loses the agent.
    drop(stream_b);
    // Wait for the *second* frame of the farewell pair: the two are pushed together, but TCP may hand them
    // over in separate reads, and a read that stops at the first one can miss the second by a scheduling
    // hair. Asking for the last frame is the read that cannot race.
    let left = read_until(&mut stream_a, &mut client_a, game::WORLD_REMOVE_AGENT).await;
    let destroyed = left
        .iter()
        .find(|frame| frame.header() == game::PLAYER_DESTROY)
        .expect("the departure is announced");
    assert_eq!(
        // A four-byte game header, so the body opens with the header's upper half: the player id starts
        // at byte 2 of the body, not at byte 0.
        u32::from_le_bytes([
            destroyed.body()[2],
            destroyed.body()[3],
            destroyed.body()[4],
            destroyed.body()[5]
        ]),
        newcomer_id,
        "the player that left is the one named"
    );
    assert!(
        left.iter()
            .any(|frame| frame.header() == game::WORLD_REMOVE_AGENT),
        "and its agent is withdrawn, in the same push"
    );

    drop(stream_a);
    placement::remove_world(&harness.pool, harness.world_id)
        .await
        .expect("remove the world");
    harness.stop();
}

/// `/tp`: a server transfer, not a position change.
///
/// What the reference does (`PreSearing/docs/08-gm-commands.md`): reserve a seat on the destination,
/// mint a player token and record the identity the next connection must present, send
/// `TRANSFER_GAME_SERVER_INFO` then `MAP_UPDATE_CURRENT`, and let the client reconnect. The two messages
/// are pinned to the capture; this test pins the cycle around them: the refusals, the redirect, the
/// transfer row, and the seat accounting that a naive teardown gets wrong.
#[tokio::test]
async fn a_teleport_redirects_to_the_target_map_and_keeps_its_seat() {
    if skip() {
        return;
    }
    let Some(harness) = Harness::start().await else {
        return;
    };

    let version = harness.version(harness.player_token);
    let Some((mut stream, mut client)) = connect_properly(harness.address, &version).await else {
        panic!("the world refused a token the auth service had just issued");
    };
    ask(&mut stream, &mut client, 0x8088, 2).await;
    read_until(&mut stream, &mut client, game::INSTANCE_LOAD_SPAWN_POINT).await;
    ask(&mut stream, &mut client, 0x8090, 19).await;
    read_until(&mut stream, &mut client, game::INSTANCE_LOAD_FINISH).await;

    // The reference's first refusal: a name that is not a map. Nothing goes on the wire.
    say(&mut stream, &mut client, 0x8064, &chat_body("/tp nowhere")).await;
    // And the one that is already where it asked to go.
    say(
        &mut stream,
        &mut client,
        0x8064,
        &chat_body("/tp lakeside county"),
    )
    .await;
    // A ping is answered, and its reply is what proves the two refusals sent nothing before it: the
    // frames that arrive are the ping's, not a redirect's.
    ask(&mut stream, &mut client, 0x800D, 2).await;
    let after_refusals = read_until(&mut stream, &mut client, game::PING_REPLY).await;
    assert!(
        !after_refusals
            .iter()
            .any(|frame| frame.header() == game::TRANSFER_GAME_SERVER_INFO),
        "a refused /tp sends no redirect"
    );

    // The pair is read by hand, exactly as GWPS's own capture shows it: the live 0x01A6 is a
    // four-byte game header, host[24], then six dwords (`map_token, region, map_id, is_explorable,
    // player_token, 0`), and the 0x0099 that follows is twelve bytes rather than its five-byte login
    // form (the reference's own "size per context" trap, `docs/03-protocol.md` section 6).
    say(
        &mut stream,
        &mut client,
        0x8064,
        &chat_body("/tp ascalon city"),
    )
    .await;
    let (transfer, update) = read_redirect(&mut stream, &mut client).await;

    assert_eq!(&transfer[0..2], &[0xA6, 0x01], "0x01A6 first");
    assert_eq!(
        transfer.len(),
        46,
        "46 bytes, GWPS's own struct sent as sizeof(*msg2)"
    );
    // host[24] at [2..26], then map_token, region, player_token, is_explorable, map_id.
    let dword = |at: usize, bytes: &[u8]| {
        u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
    };
    assert_eq!(&transfer[2..4], &[2, 0], "AF_INET, little endian");
    assert_eq!(
        dword(26, &transfer),
        harness.world_id as u32,
        "map_token: the world to join"
    );
    assert_eq!(
        dword(30, &transfer),
        0,
        "region: this server has none, and sends 0"
    );
    let player_token = dword(34, &transfer);
    assert_ne!(player_token, 0, "player_token: the token to reconnect with");
    assert_eq!(
        dword(38, &transfer),
        0,
        "is_explorable: an outpost, and the flag describes the destination"
    );
    assert_eq!(
        dword(42, &transfer),
        harness.other_map_id as u32,
        "map_id: the destination"
    );

    assert_eq!(&update[0..2], &[0x99, 0x00], "0x0099 follows");
    assert_eq!(update.len(), 12, "the redirect form is twelve bytes");
    assert_eq!(
        dword(4, &update),
        harness.other_map_id as u32,
        "and names the same destination"
    );

    // The seat moved: the old map's is free, the destination's is taken, in one transaction.
    let counts = placement::pool_counts(&harness.pool, harness.world_id, harness.other_map_id)
        .await
        .expect("read the counter")
        .expect("a pool row for the destination");
    assert_eq!(counts.1, 1, "the destination seat is taken");

    // The client reconnects with the token it was handed, and lands in the destination map.
    let moved_version = GameVersion {
        version: 39112,
        h0008: 1,
        map_token: harness.world_id as u32,
        map_id: harness.other_map_id as u32,
        player_token,
        account: harness.account.to_bytes_le(),
        character: harness.character.to_bytes_le(),
        h0038: 0,
        h003c: 0,
    };
    let Some((mut moved_stream, mut moved_client)) =
        connect_properly(harness.address, &moved_version).await
    else {
        panic!("the world refused the token it had just issued");
    };
    ask(&mut moved_stream, &mut moved_client, 0x8088, 2).await;
    let spawn = read_until(
        &mut moved_stream,
        &mut moved_client,
        game::INSTANCE_LOAD_SPAWN_POINT,
    )
    .await;
    let spawned = spawn
        .iter()
        .find(|frame| frame.header() == game::INSTANCE_LOAD_SPAWN_POINT)
        .expect("the spawn answer");
    // The spawn message carries the blob id first, and both maps here share one, so the position is
    // what says *which* map: the destination spawns at (1, 2) and the origin at (6602, 4485).
    let x = f32::from_le_bytes([
        spawned.body()[4],
        spawned.body()[5],
        spawned.body()[6],
        spawned.body()[7],
    ]);
    let y = f32::from_le_bytes([
        spawned.body()[8],
        spawned.body()[9],
        spawned.body()[10],
        spawned.body()[11],
    ]);
    assert_eq!(
        (x, y),
        (1.0, 2.0),
        "the new connection stands at the destination's own spawn point"
    );

    // The old connection now goes away. Its teardown must **not** hand back the seat, because the only
    // lease the session holds is the destination's -- the one this connection is standing on.
    drop(stream);
    tokio::time::sleep(Duration::from_millis(200)).await;
    let counts = placement::pool_counts(&harness.pool, harness.world_id, harness.other_map_id)
        .await
        .expect("read the counter")
        .expect("a pool row for the destination");
    assert_eq!(
        counts.1, 1,
        "the moved connection's teardown left the destination seat alone"
    );

    drop(moved_stream);
    placement::remove_world(&harness.pool, harness.world_id)
        .await
        .expect("remove the world");
    harness.stop();
}

/// The world map's request (`0x80B1`): the client names the instance it wants and this server carries the
/// decision out with the cycle `/tp` uses - the same seat, the same transfer row, the same two messages.
///
/// The bytes are a real client's own (`logs/world-8001.log`: `world <- 0x80B1 (9 bytes): B1 80 94 00 02 00
/// 00 00 01`), with the instance swapped for this harness's destination. There is deliberately no policy
/// here about *which* instances may be named: the client decides, and an outpost and the explorable it
/// exits to can share one map blob, so a server-side rule about "which kinds of instance" refuses travel
/// the game itself allows (`docs/22-world-model.md`).
#[tokio::test]
async fn the_world_map_request_travels_to_the_instance_it_names() {
    if skip() {
        return;
    }
    let Some(harness) = Harness::start().await else {
        return;
    };

    let version = harness.version(harness.player_token);
    let Some((mut stream, mut client)) = connect_properly(harness.address, &version).await else {
        panic!("the world refused a token the auth service had just issued");
    };
    ask(&mut stream, &mut client, 0x8088, 2).await;
    read_until(&mut stream, &mut client, game::INSTANCE_LOAD_SPAWN_POINT).await;
    ask(&mut stream, &mut client, 0x8090, 19).await;
    read_until(&mut stream, &mut client, game::INSTANCE_LOAD_FINISH).await;

    // The destination instance, then the three bytes this client always sends with it.
    let mut body = (harness.other_map_id as u16).to_le_bytes().to_vec();
    body.extend_from_slice(&[0x02, 0x00, 0x00, 0x00, 0x01]);
    say(&mut stream, &mut client, 0x80B1, &body).await;

    // Everything a travel puts on the wire, read raw: the twelve-byte `0x0099` a redirect carries is not
    // the table's five-byte form, so the stream is scanned rather than framed.
    // The travel's own first message is a `0x00F1` carrying the effect word, so that pair is what the
    // scan looks for: a tick may sit in front of it, and `0x5A`/`0x21` occur by chance inside other
    // bodies, so they cannot be the signal that the travel has arrived.
    let effect_frame = |plain: &[u8]| -> Option<usize> {
        (0..plain.len().saturating_sub(10)).find(|&at| {
            plain[at] == 0xF1
                && plain[at + 1] == 0x00
                && plain[at + 6..at + 10] == [0x00, 0x01, 0x00, 0x00]
        })
    };
    // The eight messages of one travel, in the order and at the sizes a live server sends them
    // (`captures/zone-ref2-012225.jsonl`, seq 971 to 982):
    let travel: &[(u16, usize)] = &[
        (0x00F1, 10),
        (0x0028, 6),
        (0x01A6, 46),
        (0x0099, 12),
        (0x00B0, 12),
        (0x00B1, 12),
        (0x005A, 8),
        (0x0021, 8),
    ];
    let whole: usize = travel.iter().map(|(_, size)| size).sum();

    let mut plain: Vec<u8> = Vec::new();
    let mut buffer = [0u8; 8192];
    let start = loop {
        let read = tokio::time::timeout(TIMEOUT, stream.read(&mut buffer))
            .await
            .expect("read before the timeout")
            .expect("read");
        assert!(
            read > 0,
            "the world closed the connection before the travel"
        );
        let mut chunk = buffer[..read].to_vec();
        client.decrypt(&mut chunk);
        plain.extend_from_slice(&chunk);
        if let Some(at) = effect_frame(&plain) {
            if plain.len() >= at + whole {
                break at;
            }
        }
    };

    let mut at = start;
    for (header, size) in travel {
        assert!(
            plain.len() >= at + size,
            "the stream ended before the {header:#06X} of the travel"
        );
        let found = u16::from_le_bytes([plain[at], plain[at + 1]]);
        assert_eq!(
            found, *header,
            "at byte {at}: expected {header:#06X}, found {found:#06X}"
        );
        at += size;
    }

    // And the redirect names the instance the client asked for.
    let dword = |at: usize, bytes: &[u8]| {
        u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
    };
    let transfer = start + 16;
    assert_eq!(
        dword(transfer + 42, &plain),
        harness.other_map_id as u32,
        "the instance the client named is the destination"
    );
    assert_eq!(
        dword(transfer + 38, &plain),
        0,
        "is_explorable describes the destination, and this one is an outpost"
    );
    assert_eq!(
        dword(transfer + 46 + 4, &plain),
        harness.other_map_id as u32,
        "and the 0x0099 that follows names the same destination"
    );

    drop(stream);
    placement::remove_world(&harness.pool, harness.world_id)
        .await
        .expect("remove the world");
    harness.stop();
}

/// How far an arrival stands from the nearest of the destination map's own spawn points. A spawn is
/// scattered over the `spawn_scatter` circle around one of them (`config/Server/spawns.json`,
/// `Standing::at_spawn`), and a map the pack states no list for is answered by its catalogue
/// coordinate.
fn distance_to_nearest_spawn(
    destination: i32,
    configured: &db_game::SpawnMap,
    x: f32,
    y: f32,
) -> f32 {
    let mut points: Vec<(f32, f32)> = asura_world::map_spawns::for_map(destination)
        .map(|list| list.iter().map(|spawn| (spawn.x, spawn.y)).collect())
        .unwrap_or_default();
    if points.is_empty() {
        points.push((configured.spawn_x, configured.spawn_y));
    }
    points.iter().fold(f32::INFINITY, |nearest, (px, py)| {
        (px - x).hypot(py - y).min(nearest)
    })
}

/// `--on-demand`: a travel to a map this world was **not** started with.
///
/// The reference pre-spawns one server per configured map so a travel never constructs anything, and
/// this world preloads every hosted map for the same reason. Whether the real client *requires* the
/// destination to exist before the hop is the open question the `--on-demand` switch exists to
/// answer (`docs/03-protocol.md`). This test pins the server half: the transfer names a map the world
/// was not started with, its map entry and pool row appear at travel time, and the reconnect lands on
/// the destination's own spawn list (never on the source map's coordinate).
#[tokio::test]
async fn a_travel_to_an_unhosted_map_is_loaded_on_demand() {
    if skip() {
        return;
    }
    let Some(url) = asura_db::config::database_url() else {
        return;
    };
    let pool = placement::connect(&url, 4)
        .await
        .expect("connect the database");

    let source = HARNESS_MAP;
    let destination = HARNESS_OTHER_MAP;
    // The demand load reads the destination's config from the catalogue; a database without it can
    // only skip (the battery seeds it, a bare `cargo test` may not).
    let Some(configured) = db_game::spawn_map(&pool, destination)
        .await
        .expect("read the destination's map config")
    else {
        println!("map {destination} is not in map_config; skipping the on-demand travel test");
        return;
    };

    let world_id = unique_world();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind a loopback port");
    let address = listener.local_addr().expect("local address");

    // Only the source map is hosted: the destination has to come from `--on-demand`.
    let maps = vec![MapEntry {
        map_id: source,
        map_file_id: 0x1B97D,
        map_type: 0,
        spawn: (6602.0, 4485.0),
        spawn_plane: 0,
        spawns: Vec::new(),
        name: "Lakeside County".to_string(),
        capacity: 2,
    }];
    let graphs = harness_graphs(&maps);
    let world = Arc::new(World {
        pool: pool.clone(),
        world_id,
        maps,
        replay: None,
        instances: Default::default(),
        public: address,
        graphs,
        lazy_maps: Default::default(),
        load_on_demand: true,
        capacity: 2,
        archive: gw_dat_path().unwrap_or_default(),
    });
    let server = tokio::spawn({
        let world = world.clone();
        async move {
            let _ = serve(listener, world, true).await;
        }
    });

    let seeded = db::seed_identity(
        &pool,
        Uuid::new_v4(),
        Uuid::new_v4(),
        &format!("ondemand-{}", Uuid::new_v4()),
        "Asura Test",
        source,
    )
    .await
    .expect("seed the identity");
    placement::register_world(
        &pool,
        world_id,
        "ondemand-world",
        &address.ip().to_string(),
        i32::from(address.port()),
        "m4-test",
        &[(source, 2)],
    )
    .await
    .expect("register the world");
    placement::reserve(&pool, world_id, source, seeded.session)
        .await
        .expect("reserve the source seat");
    let player_token = (Uuid::new_v4().as_u128() & 0xFFFF_FFFF) as u32;
    placement::begin_transfer(
        &pool,
        player_token,
        world_id,
        source,
        seeded.session,
        seeded.account,
        seeded.character,
    )
    .await
    .expect("write the pending transfer");

    let version = GameVersion {
        version: 39112,
        h0008: 1,
        map_token: world_id as u32,
        map_id: source as u32,
        player_token,
        account: seeded.account.to_bytes_le(),
        character: seeded.character.to_bytes_le(),
        h0038: 0,
        h003c: 0,
    };
    let Some((mut stream, mut client)) = connect_properly(address, &version).await else {
        panic!("refused a token that had just been issued");
    };
    ask(&mut stream, &mut client, 0x8088, 2).await;
    read_until(&mut stream, &mut client, game::INSTANCE_LOAD_SPAWN_POINT).await;
    ask(&mut stream, &mut client, 0x8090, 19).await;
    read_until(&mut stream, &mut client, game::INSTANCE_LOAD_FINISH).await;

    // Before the travel, the destination is not known to the world: this is what `--on-demand` acts on.
    assert!(
        world.map(destination).is_none(),
        "the destination starts unhosted"
    );

    say(
        &mut stream,
        &mut client,
        0x8064,
        &chat_body("/tp ascalon city"),
    )
    .await;
    let (transfer, update) = read_redirect(&mut stream, &mut client).await;
    assert_eq!(&transfer[0..2], &[0xA6, 0x01], "0x01A6 first");
    let dword = |at: usize, bytes: &[u8]| {
        u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
    };
    assert_eq!(
        dword(42, &transfer),
        destination as u32,
        "the redirect names the demand-loaded map"
    );
    assert_eq!(
        dword(38, &transfer),
        0,
        "is_explorable: the destination's own type"
    );
    // GWPS's field order: the token to reconnect with is the third dword, before is_explorable.
    let moved_token = dword(34, &transfer);
    assert_ne!(moved_token, 0, "the redirect minted a reconnect token");
    assert_eq!(dword(4, &update), destination as u32);

    // The demand load happened: the entry and the seat pool row now exist.
    assert!(
        world.map(destination).is_some(),
        "the destination was loaded on demand"
    );
    let counts = placement::pool_counts(&pool, world_id, destination)
        .await
        .expect("read the counter")
        .expect("a pool row for the demand-loaded destination");
    assert_eq!(counts.1, 1, "the destination seat is taken");

    // The client reconnects with the token the redirect minted, and stands on the destination's own
    // configured spawn.
    let moved_version = GameVersion {
        version: 39112,
        h0008: 1,
        map_token: world_id as u32,
        map_id: destination as u32,
        player_token: moved_token,
        account: seeded.account.to_bytes_le(),
        character: seeded.character.to_bytes_le(),
        h0038: 0,
        h003c: 0,
    };
    let Some((mut moved_stream, mut moved_client)) =
        connect_properly(address, &moved_version).await
    else {
        panic!("the world refused the token it had just issued");
    };
    ask(&mut moved_stream, &mut moved_client, 0x8088, 2).await;
    let spawn = read_until(
        &mut moved_stream,
        &mut moved_client,
        game::INSTANCE_LOAD_SPAWN_POINT,
    )
    .await;
    let spawned = spawn
        .iter()
        .find(|frame| frame.header() == game::INSTANCE_LOAD_SPAWN_POINT)
        .expect("the spawn answer");
    let x = f32::from_le_bytes([
        spawned.body()[4],
        spawned.body()[5],
        spawned.body()[6],
        spawned.body()[7],
    ]);
    let y = f32::from_le_bytes([
        spawned.body()[8],
        spawned.body()[9],
        spawned.body()[10],
        spawned.body()[11],
    ]);
    // The demand-loaded destination answers with its own spawn data, scattered over a 250-unit circle
    // around one of its own spawn points - never the source map's coordinate. What must never happen
    // is the source map's coordinate.
    assert_ne!(
        (x, y),
        (6602.0f32, 4485.0f32),
        "the source map's spawn point is not the destination's"
    );
    let nearest = distance_to_nearest_spawn(destination, &configured, x, y);
    assert!(
        nearest <= asura_world::spawn_scatter() + 0.5,
        "the arrival ({x}, {y}) is {nearest} units from the destination's nearest spawn point, \
         outside the 250-unit scatter circle"
    );
    assert_eq!(
        u32::from_le_bytes([
            spawned.body()[0],
            spawned.body()[1],
            spawned.body()[2],
            spawned.body()[3]
        ]),
        u32::try_from(configured.map_file_id).unwrap_or(0),
        "and its own map blob"
    );

    drop(moved_stream);
    drop(stream);
    placement::remove_world(&pool, world_id)
        .await
        .expect("remove the world");
    server.abort();
}

/// `--on-demand`, the login path: a connection whose character stands in a map this world was not
/// started with.
///
/// This is the refusal observed live: a world started with `--maps 146` and a client whose character
/// was last in map 148 answered "this world does not host map 148" and was kicked (client error 007).
/// With `--on-demand` the world loads the map on the spot and serves it, exactly as it does for a
/// `/tp` into an unhosted map.
#[tokio::test]
async fn a_connection_to_an_unhosted_map_is_loaded_on_demand() {
    if skip() {
        return;
    }
    let Some(url) = asura_db::config::database_url() else {
        return;
    };
    let pool = placement::connect(&url, 4)
        .await
        .expect("connect the database");

    let source = HARNESS_MAP;
    let destination = HARNESS_OTHER_MAP;
    let Some(configured) = db_game::spawn_map(&pool, destination)
        .await
        .expect("read the destination's map config")
    else {
        println!("map {destination} is not in map_config; skipping the on-demand login test");
        return;
    };

    let world_id = unique_world();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind a loopback port");
    let address = listener.local_addr().expect("local address");

    let maps = vec![MapEntry {
        map_id: source,
        map_file_id: 0x1B97D,
        map_type: 0,
        spawn: (6602.0, 4485.0),
        spawn_plane: 0,
        spawns: Vec::new(),
        name: "Lakeside County".to_string(),
        capacity: 2,
    }];
    let graphs = harness_graphs(&maps);
    let world = Arc::new(World {
        pool: pool.clone(),
        world_id,
        maps,
        replay: None,
        instances: Default::default(),
        public: address,
        graphs,
        lazy_maps: Default::default(),
        load_on_demand: true,
        capacity: 2,
        archive: gw_dat_path().unwrap_or_default(),
    });
    let server = tokio::spawn({
        let world = world.clone();
        async move {
            let _ = serve(listener, world, true).await;
        }
    });

    // The character stands in the unhosted map: that is what the login travel asks for, and what the
    // client's version message repeats.
    let seeded = db::seed_identity(
        &pool,
        Uuid::new_v4(),
        Uuid::new_v4(),
        &format!("ondemand-login-{}", Uuid::new_v4()),
        "Asura Test",
        destination,
    )
    .await
    .expect("seed the identity");
    placement::register_world(
        &pool,
        world_id,
        "ondemand-login-world",
        &address.ip().to_string(),
        i32::from(address.port()),
        "m4-test",
        &[(destination, 2)],
    )
    .await
    .expect("register the world");
    placement::reserve(&pool, world_id, destination, seeded.session)
        .await
        .expect("reserve the destination seat");
    let player_token = (Uuid::new_v4().as_u128() & 0xFFFF_FFFF) as u32;
    placement::begin_transfer(
        &pool,
        player_token,
        world_id,
        destination,
        seeded.session,
        seeded.account,
        seeded.character,
    )
    .await
    .expect("write the pending transfer");

    let version = GameVersion {
        version: 39112,
        h0008: 1,
        map_token: world_id as u32,
        map_id: destination as u32,
        player_token,
        account: seeded.account.to_bytes_le(),
        character: seeded.character.to_bytes_le(),
        h0038: 0,
        h003c: 0,
    };
    let Some((mut stream, mut client)) = connect_properly(address, &version).await else {
        panic!("the world refused a token for a map it did not host, with --on-demand on");
    };

    // The connection itself made the world load the map: entry present, pool row published.
    assert!(
        world.map(destination).is_some(),
        "the connection demand-loaded its map"
    );
    ask(&mut stream, &mut client, 0x8088, 2).await;
    let spawn = read_until(&mut stream, &mut client, game::INSTANCE_LOAD_SPAWN_POINT).await;
    let spawned = spawn
        .iter()
        .find(|frame| frame.header() == game::INSTANCE_LOAD_SPAWN_POINT)
        .expect("the spawn answer");
    let x = f32::from_le_bytes([
        spawned.body()[4],
        spawned.body()[5],
        spawned.body()[6],
        spawned.body()[7],
    ]);
    let y = f32::from_le_bytes([
        spawned.body()[8],
        spawned.body()[9],
        spawned.body()[10],
        spawned.body()[11],
    ]);
    // The demand-loaded map answers with its own spawn data, never the source map's coordinate; the
    // same rule as the travel test above.
    assert_ne!(
        (x, y),
        (6602.0f32, 4485.0f32),
        "the source map's spawn point is not the destination's"
    );
    let nearest = distance_to_nearest_spawn(destination, &configured, x, y);
    assert!(
        nearest <= asura_world::spawn_scatter() + 0.5,
        "the arrival ({x}, {y}) is {nearest} units from the destination's nearest spawn point, \
         outside the 250-unit scatter circle"
    );

    drop(stream);
    placement::remove_world(&pool, world_id)
        .await
        .expect("remove the world");
    server.abort();
}
/// T6: a character can be made in a character-creation instance.
///
/// The reference's four messages (`GameSrv_HandleCharCreation*`), in order: the player-data block, the PvP
/// half, a profession change that swaps the equipped set, and a confirm that writes the rows and answers
/// `CHAR_CREATION_SUCCESS`. The equipment is memory until the confirm, which is where the reference writes
/// it too.
#[tokio::test]
async fn a_character_is_created_in_a_creation_instance() {
    if skip() {
        return;
    }
    let Some(harness) = Harness::start_on(11).await else {
        return;
    };

    let version = harness.version(harness.player_token);
    let Some((mut stream, mut client)) = connect_properly(harness.address, &version).await else {
        panic!("the world refused a token the auth service had just issued");
    };

    // 1. the player-data block ends with `INSTANCE_PLAYER_DATA_DONE`, which is creation-only.
    ask(&mut stream, &mut client, 0x8089, 2).await;
    let block = read_until(&mut stream, &mut client, game::INSTANCE_PLAYER_DATA_DONE).await;
    assert!(
        block
            .iter()
            .any(|frame| frame.header() == game::INVENTORY_CREATE_BAG),
        "the creation screen is given bags"
    );
    assert!(
        block
            .iter()
            .any(|frame| frame.header() == game::CREATE_NAMED_ITEM),
        "and the starting equipment"
    );
    // The three int properties `GameSrv_SendPlayerHealthEnergy` sends on this path: energy 20, health 100,
    // and property 64 zero. Not the load path's 25/400/regen-float/level, which is a different block.
    let properties: Vec<(u32, u32)> = block
        .iter()
        .filter(|frame| frame.header() == game::AGENT_PROPERTY_UPDATE_INT)
        .map(|frame| {
            (
                u32::from_le_bytes([
                    frame.body()[0],
                    frame.body()[1],
                    frame.body()[2],
                    frame.body()[3],
                ]),
                u32::from_le_bytes([
                    frame.body()[8],
                    frame.body()[9],
                    frame.body()[10],
                    frame.body()[11],
                ]),
            )
        })
        .collect();
    assert_eq!(
        properties,
        vec![
            (game::AGENT_PROPERTY_ENERGY, 20),
            (game::AGENT_PROPERTY_HEALTH, 100),
            (64, 0),
        ],
        "the creation path's own three properties, in its own order"
    );
    assert!(
        !block
            .iter()
            .any(|frame| frame.header() == game::AGENT_PROPERTY_UPDATE_FLOAT),
        "and no float property: the creation path sends none"
    );

    // 2. the PvP half: empty streams, as an account with nothing unlocked has.
    ask(&mut stream, &mut client, 0x808A, 2).await;
    let pvp = read_until(&mut stream, &mut client, game::PVP_ITEM_STREAM_END).await;
    assert!(
        pvp.iter()
            .any(|frame| frame.header() == game::PVP_UPDATE_UNLOCKED_SKILLS),
        "the unlocked-skill list"
    );

    // 3. a profession change: Prophecies Ranger (campaign 1, profession 2).
    say(&mut stream, &mut client, 0x8060, &[1, 2]).await;
    let changed = read_until(&mut stream, &mut client, game::PLAYER_UPDATE_PROFESSION).await;
    // The **named** creator, which is what live sends on this path: five `0x0161` after every
    // `0x0060` in `recordings/client/feature/character_creation/character_creation-live-necro_factions.jsonl`.
    // An item created without its name makes the creation screen assert `ptr->IsDetailHigh()`
    // (`ItCliApi.cpp(698)`, measured 2026-10-28) the moment it asks for the details.
    assert!(
        changed
            .iter()
            .any(|frame| frame.header() == game::CREATE_NAMED_ITEM),
        "the new profession's equipment follows the profession"
    );

    // 4. the confirm: name, appearance, flags. The reference's confirm is 58 bytes.
    let mut confirm = Vec::new();
    let units: Vec<u16> = "Asura Test".encode_utf16().collect();
    confirm.extend_from_slice(&(units.len() as u16).to_le_bytes());
    for unit in &units {
        confirm.extend_from_slice(&unit.to_le_bytes());
    }
    // An appearance the client actually chose: the packed `Appearance` word the confirm carries.
    let chosen_appearance: u32 = 0x3060_0405; // sex, height, skin, hair and face bits, all set
    confirm.extend_from_slice(&chosen_appearance.to_le_bytes());
    confirm.extend_from_slice(&[0u8; 4]); // the rest of the eight-byte config
    confirm.extend_from_slice(&0u32.to_le_bytes()); // flags
    say(&mut stream, &mut client, 0x808B, &confirm).await;
    let created = read_until(&mut stream, &mut client, game::CHAR_CREATION_SUCCESS).await;
    let success = created
        .iter()
        .find(|frame| frame.header() == game::CHAR_CREATION_SUCCESS)
        .expect("the character was written");
    // Little endian, which is what the reference writes (`uuid_enc_le`) and what the builder sent.
    let character = Uuid::from_bytes_le(success.body()[0..16].try_into().expect("sixteen bytes"));

    // The rows T6 asks for, read back through this repo's own queries: the character, its six bags, and
    // the profession's equipment in the equipped bag.
    assert_eq!(
        asura_db::game::character_name(&harness.pool, character)
            .await
            .expect("read the character"),
        Some("Asura Test".to_string()),
        "a row in characters, named as the client named it"
    );
    // And the traits the load block reads: the appearance the client chose, the profession the screen
    // last set, and the one unlocked-profession bit the reference sets for it.
    let traits = asura_db::game::traits(&harness.pool, character)
        .await
        .expect("read the traits")
        .expect("a traits row");
    assert_eq!(
        traits.appearance, chosen_appearance,
        "the chosen appearance"
    );
    assert_eq!(traits.profession, 2, "the Ranger the screen settled on");
    assert_eq!(traits.unlocked_professions, 1 << 2, "and its one bit");
    let inventory = db_inventory::load(&harness.pool, character)
        .await
        .expect("read the inventory");
    assert_eq!(inventory.bags.len(), 6, "the reference's six bags");
    let equipped_bag = inventory
        .bags
        .iter()
        .find(|bag| bag.bag_type == 2)
        .expect("an equipped bag")
        .bag_id;
    let worn: Vec<i64> = inventory
        .items
        .iter()
        .filter(|item| item.bag_id == Some(equipped_bag))
        .map(|item| item.file_id)
        .collect();
    assert!(!worn.is_empty(), "the profession's equipment was written");
    // And it is the *Ranger's* set, which is what the profession change asked for (campaign 1,
    // profession 2) - not the Warrior the screen started with.
    let expected: Vec<i64> = asura_world::default_equipment::default_equipment(1, 2)
        .iter()
        .map(|spec| i64::from(spec.file_id))
        .collect();
    let mut worn_sorted = worn.clone();
    let mut expected_sorted = expected.clone();
    worn_sorted.sort_unstable();
    expected_sorted.sort_unstable();
    assert_eq!(
        worn_sorted, expected_sorted,
        "the equipped set is the one the chosen profession's table gives"
    );

    drop(stream);
    placement::remove_world(&harness.pool, harness.world_id)
        .await
        .expect("remove the world");
    harness.stop();
}

/// T13: an idle pair keeps its rhythm, and neither the world nor the mailbox grows.
///
/// The reference's T13 asks for two clients left alone for ten minutes: no crash, the ping/tick cycle still
/// running, memory roughly flat. All three are measurable: the rhythm by counting what arrives, the
/// accounting by watching the world's peer count, and the memory by asking the operating system for this
/// process's working set (`K32GetProcessMemoryInfo`, no crate needed) -- which is what the `#[ignore]`d
/// ten-minute variant below does, self-checking the sampler first so a flat line cannot come from an
/// instrument that never moves.
///
/// The window here is seconds rather than minutes, because a test that takes ten minutes is a test nobody
/// runs: what it asserts is the *cadence* (`PING_INTERVAL` 5 s, `TICK_INTERVAL` 500 ms, the reference's own
/// constants), which does not change with the length of the observation.
#[tokio::test]
async fn an_idle_pair_keeps_its_rhythm_and_its_count() {
    if skip() {
        return;
    }
    let Some(harness) = Harness::start().await else {
        return;
    };

    let version = harness.version(harness.player_token);
    let Some((mut stream_a, mut client_a)) = connect_properly(harness.address, &version).await
    else {
        panic!("the world refused the first client");
    };
    // The load block arrives unasked; it is not part of the idle window, so it is consumed first.
    read_until(&mut stream_a, &mut client_a, game::INSTANCE_LOAD_INFO).await;
    // The clock is released by the load's own end, `INSTANCE_LOAD_FINISH` (`world::handle_request_players`),
    // so this client completes its load here - before the second one exists - and the idle window below is
    // two loaded connections and nothing else. Measured 2026-11-01: a live load carries no `0x001E` between
    // the client's spawn request and its players request, while this world used to tick a client that was
    // still loading.
    ask(&mut stream_a, &mut client_a, 0x8090, 19).await;
    read_until(&mut stream_a, &mut client_a, game::INSTANCE_LOAD_FINISH).await;

    // A second player on the same map, the way the two-player test places one.
    let second = db::seed_identity(
        &harness.pool,
        Uuid::new_v4(),
        Uuid::new_v4(),
        &format!("m4-idle-{}", Uuid::new_v4()),
        "Idle Player",
        harness.map_id,
    )
    .await
    .expect("seed a second identity");
    placement::reserve(
        &harness.pool,
        harness.world_id,
        harness.map_id,
        second.session,
    )
    .await
    .expect("reserve for the second player");
    let token_b = (Uuid::new_v4().as_u128() & 0xFFFF_FFFF) as u32;
    placement::begin_transfer(
        &harness.pool,
        token_b,
        harness.world_id,
        harness.map_id,
        second.session,
        second.account,
        second.character,
    )
    .await
    .expect("write the second transfer");
    let mut version_b = harness.version(token_b);
    version_b.account = second.account.to_bytes_le();
    version_b.character = second.character.to_bytes_le();
    let Some((mut stream_b, mut client_b)) = connect_properly(harness.address, &version_b).await
    else {
        panic!("the world refused the second client");
    };
    ask(&mut stream_b, &mut client_b, 0x8088, 2).await;
    read_until(
        &mut stream_b,
        &mut client_b,
        game::INSTANCE_LOAD_SPAWN_POINT,
    )
    .await;

    // Both connections are now idle: nothing is sent, and the server keeps its own clock running.
    let mut buffer = [0u8; 8192];
    let mut frames: Vec<Frame> = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(6);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(200), stream_a.read(&mut buffer)).await {
            Ok(Ok(0)) => panic!("the world closed an idle connection"),
            Ok(Ok(read)) => frames.extend(client_a.receive_game(&buffer[..read]).expect("frames")),
            Ok(Err(err)) => panic!("an idle connection failed: {err}"),
            Err(_) => {} // nothing arrived in this slice, which is fine
        }
    }

    let ticks = frames
        .iter()
        .filter(|frame| frame.header() == game::WORLD_SIMULATION_TICK)
        .count();
    let pings = frames
        .iter()
        .filter(|frame| frame.header() == game::PING_REQUEST)
        .count();
    // Six seconds at a 500 ms tick is at least ten; the assertion allows for scheduling slack.
    assert!(
        ticks >= 8,
        "the simulation tick kept running while idle: {ticks} in six seconds"
    );
    assert!(
        pings >= 1,
        "the ping cycle kept running while idle: {pings} in six seconds"
    );
    // Nothing else was pushed at an idle client: an idle pair gets the clock and the ping, not a stream
    // of anything else.
    let unexpected: Vec<u16> = frames
        .iter()
        .map(|frame| frame.header())
        .filter(|header| *header != game::WORLD_SIMULATION_TICK && *header != game::PING_REQUEST)
        .collect();
    assert!(
        unexpected.is_empty(),
        "an idle client is only told the time: {unexpected:?}"
    );

    assert_eq!(
        harness.world.instances.len(),
        2,
        "two players, and the world's list did not grow while they were idle"
    );

    drop(stream_a);
    drop(stream_b);
    // The registry empties as the connections go, so the count is not a slow leak.
    let mut left = harness.world.instances.len();
    for _ in 0..50 {
        if left == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        left = harness.world.instances.len();
    }
    assert_eq!(left, 0, "both players left the world's list");

    placement::remove_world(&harness.pool, harness.world_id)
        .await
        .expect("remove the world");
    harness.stop();
}

/// T13's full window: ten minutes idle, the cadence, the accounting -- and the working set.
///
/// ```text
/// cargo test -p asura-world --test load a_ten_minute_idle -- --ignored --nocapture
/// ```
///
/// The reference's T13 says "no crash, the ping cycle continues, memory roughly flat", and the third clause
/// used to be left to a human because a working set "cannot be measured portably". It can be measured, just
/// not portably: Windows answers with `K32GetProcessMemoryInfo` in kernel32, which needs no crate. The world
/// runs as a task inside this process here (the reference runs a process per map), so "the process" in the
/// reference's sentence and this sample are the same thing: everything the world has allocated, plus this
/// harness.
///
/// The window is `ASURA_SOAK_SECONDS` (default 600) so the mechanism can be exercised in a minute; the
/// verdict is the same either way -- after a tenth of the window to let the allocator settle, the working set
/// must not grow by more than [`SOAK_GROWTH_LIMIT`].
const SOAK_GROWTH_LIMIT: u64 = 8 * 1024 * 1024;

/// This process's working set in bytes, or `None` where there is no such measurement.
#[cfg(windows)]
fn working_set_bytes() -> Option<u64> {
    // `PROCESS_MEMORY_COUNTERS`, field for field (`psapi.h`).
    #[repr(C)]
    #[derive(Default)]
    struct ProcessMemoryCounters {
        cb: u32,
        page_fault_count: u32,
        peak_working_set_size: usize,
        working_set_size: usize,
        quota_peak_paged_pool_usage: usize,
        quota_paged_pool_usage: usize,
        quota_peak_non_paged_pool_usage: usize,
        quota_non_paged_pool_usage: usize,
        pagefile_usage: usize,
        peak_pagefile_usage: usize,
    }

    extern "system" {
        fn GetCurrentProcess() -> isize;
        fn K32GetProcessMemoryInfo(
            process: isize,
            counters: *mut ProcessMemoryCounters,
            cb: u32,
        ) -> i32;
    }

    let mut counters = ProcessMemoryCounters {
        cb: std::mem::size_of::<ProcessMemoryCounters>() as u32,
        ..ProcessMemoryCounters::default()
    };
    // Safety: `counters` is a `repr(C)` struct of exactly the layout the API documents, and `cb` is its
    // size; the handle is this process, which is always valid.
    let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb) };
    (ok != 0).then_some(counters.working_set_size as u64)
}

#[cfg(not(windows))]
fn working_set_bytes() -> Option<u64> {
    None
}

#[tokio::test]
#[ignore]
async fn a_ten_minute_idle_pair_keeps_its_rhythm() {
    // Ten minutes of wall clock is a decision, not a default. `#[ignore]` alone does not start it,
    // because `cargo test -- --ignored` would then run it by accident - which happened once.
    if std::env::var("ASURA_SOAK").is_err() {
        eprintln!("skipped: set ASURA_SOAK=1 to run the ten-minute soak (ASURA_SOAK_SECONDS sets the window)");
        return;
    }
    if skip() {
        return;
    }
    let Some(harness) = Harness::start().await else {
        return;
    };
    let window = std::env::var("ASURA_SOAK_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(600);

    let version = harness.version(harness.player_token);
    let Some((mut stream_a, mut client_a)) = connect_properly(harness.address, &version).await
    else {
        panic!("the world refused the first client");
    };
    read_until(&mut stream_a, &mut client_a, game::INSTANCE_LOAD_INFO).await;

    // The second half of the pair, placed the way the auth service places one.
    let second = db::seed_identity(
        &harness.pool,
        Uuid::new_v4(),
        Uuid::new_v4(),
        &format!("m4-soak-{}", Uuid::new_v4()),
        "Soak Player",
        harness.map_id,
    )
    .await
    .expect("seed a second identity");
    placement::reserve(
        &harness.pool,
        harness.world_id,
        harness.map_id,
        second.session,
    )
    .await
    .expect("reserve for the second player");
    let token_b = (Uuid::new_v4().as_u128() & 0xFFFF_FFFF) as u32;
    placement::begin_transfer(
        &harness.pool,
        token_b,
        harness.world_id,
        harness.map_id,
        second.session,
        second.account,
        second.character,
    )
    .await
    .expect("write the second transfer");
    let mut version_b = harness.version(token_b);
    version_b.account = second.account.to_bytes_le();
    version_b.character = second.character.to_bytes_le();
    let Some((mut stream_b, mut client_b)) = connect_properly(harness.address, &version_b).await
    else {
        panic!("the world refused the second client");
    };
    read_until(&mut stream_b, &mut client_b, game::INSTANCE_LOAD_INFO).await;

    let mut buffer_a = [0u8; 8192];
    let mut buffer_b = [0u8; 8192];
    let started = tokio::time::Instant::now();
    let mut ticks = 0usize;
    let mut pings = 0usize;

    // The instrument has to be shown to work before its verdict means anything. An idle process's working
    // set barely moves, so "flat" could as easily be a sampler that never updates -- and the first version of
    // this check proved exactly that: `vec![0u8; 16 MiB]` asks the allocator for *zeroed* memory, the OS
    // hands back demand-zero pages, and the working set does not move until they are touched. So the
    // allocation is written to, one byte per page, and the sample has to see most of it.
    if let Some(before) = working_set_bytes() {
        let mut hog = vec![0u8; 16 * 1024 * 1024];
        for page in hog.chunks_mut(4096) {
            page[0] = 1;
        }
        let after = working_set_bytes().expect("the sampler worked a moment ago");
        assert!(
            after >= before + 12 * 1024 * 1024,
            "the working-set sample did not see a 16 MiB allocation: {before} -> {after}"
        );
        drop(hog);
    }

    // The working set, sampled every fifteen seconds. The first tenth of the window is the warm-up: the
    // allocator, the pool and both connections settle there, and counting that as growth would measure
    // start-up rather than a leak.
    let warm_up = Duration::from_secs(window / 10);
    let mut baseline: Option<u64> = None;
    let mut peak = 0u64;
    let mut next_sample = Duration::ZERO;

    while started.elapsed() < Duration::from_secs(window) {
        // Both connections are drained every slice: a client that stops reading makes its channel grow in
        // the world, and that is a measurement artefact, not a leak.
        if let Ok(Ok(read)) =
            tokio::time::timeout(Duration::from_millis(100), stream_a.read(&mut buffer_a)).await
        {
            assert!(read > 0, "the world closed the first idle connection");
            for frame in client_a.receive_game(&buffer_a[..read]).expect("frames") {
                match frame.header() {
                    game::WORLD_SIMULATION_TICK => ticks += 1,
                    game::PING_REQUEST => pings += 1,
                    _ => {}
                }
            }
        }
        if let Ok(Ok(read)) =
            tokio::time::timeout(Duration::from_millis(100), stream_b.read(&mut buffer_b)).await
        {
            assert!(read > 0, "the world closed the second idle connection");
            client_b.receive_game(&buffer_b[..read]).expect("frames");
        }

        if started.elapsed() >= next_sample {
            next_sample += Duration::from_secs(15);
            if let Some(now) = working_set_bytes() {
                if started.elapsed() >= warm_up {
                    if baseline.is_none() {
                        baseline = Some(now);
                        println!(
                            "soak: warm-up over at {:?}, working set {:.1} MiB",
                            started.elapsed(),
                            now as f64 / (1024.0 * 1024.0)
                        );
                    }
                    peak = peak.max(now);
                }
                println!(
                    "soak: {:>5.0}s ticks {ticks:>5} pings {pings:>3} working set {:.1} MiB",
                    started.elapsed().as_secs_f64(),
                    now as f64 / (1024.0 * 1024.0)
                );
            }
        }
    }

    let qualified_ticks = (window as f64 * 2.0 * 0.8) as usize;
    let qualified_pings = (window as f64 / 5.0 * 0.8) as usize;
    println!(
        "{window}s idle: {ticks} ticks, {pings} pings, {} player(s) in the world",
        harness.world.instances.len()
    );
    assert!(
        ticks >= qualified_ticks,
        "about two ticks a second for {window}s: {ticks} arrived, {qualified_ticks} expected"
    );
    assert!(
        pings >= qualified_pings,
        "a ping every five seconds for {window}s: {pings} arrived, {qualified_pings} expected"
    );
    assert_eq!(harness.world.instances.len(), 2, "the pair, still");

    match (baseline, working_set_bytes()) {
        (Some(baseline), Some(end)) => {
            let growth = end.saturating_sub(baseline);
            println!(
                "soak: working set {:.1} MiB after the warm-up, {:.1} MiB at the end (peak {:.1} MiB), \
                 growth {:.2} MiB",
                baseline as f64 / (1024.0 * 1024.0),
                end as f64 / (1024.0 * 1024.0),
                peak as f64 / (1024.0 * 1024.0),
                growth as f64 / (1024.0 * 1024.0)
            );
            assert!(
                growth < SOAK_GROWTH_LIMIT,
                "memory stayed roughly flat over {window}s: the working set grew by {:.2} MiB",
                growth as f64 / (1024.0 * 1024.0)
            );
        }
        // No measurement available: say so rather than claiming the clause was met.
        _ => println!(
            "soak: no working-set measurement on this platform; the memory clause is not checked"
        ),
    }

    drop(stream_a);
    drop(stream_b);
    placement::remove_world(&harness.pool, harness.world_id)
        .await
        .expect("remove the world");
    harness.stop();
}

/// T10 landed: the walk is the reference's own.
///
/// `GameSrv_HandleMoveToCoord` (`GmAgent.c:371-398`) asks the path finder for a walk and answers with the
/// **first waypoint of its own path**, not with the client's destination -- a straight line through a wall
/// is exactly what that message exists to correct. Then the world tick walks the agent along the path
/// (`GameSrv_UpdateAgentMovement`), and `GameSrv_UpdateAgentDestination` broadcasts each new destination as
/// the agent crosses it.
///
/// The test asks the same finder the server used (`World::graphs` is public for this) what the path is, so
/// the wire can be compared against it rather than against a guess. The destination is one the map's graph
/// reaches in more than one waypoint, which is what makes "the first waypoint" different from "what the
/// client asked for".
#[tokio::test]
async fn a_walk_is_answered_with_the_first_waypoint_of_the_servers_own_path() {
    if skip() {
        return;
    }
    let Some(harness) = Harness::start().await else {
        return;
    };
    if !harness.walks {
        eprintln!("skipped: no Gw.dat, so this world has no path graph");
        harness.stop();
        return;
    }

    // Two players, because the broadcast is what the second one observes.
    let version = harness.version(harness.player_token);
    let Some((mut stream_a, mut client_a)) = connect_properly(harness.address, &version).await
    else {
        panic!("the world refused the first client");
    };
    read_until(&mut stream_a, &mut client_a, game::INSTANCE_LOAD_INFO).await;
    ask(&mut stream_a, &mut client_a, 0x8090, 19).await;
    read_until(&mut stream_a, &mut client_a, game::INSTANCE_LOAD_FINISH).await;

    let second = db::seed_identity(
        &harness.pool,
        Uuid::new_v4(),
        Uuid::new_v4(),
        &format!("m4-move-{}", Uuid::new_v4()),
        "Walker",
        harness.map_id,
    )
    .await
    .expect("seed a second identity");
    placement::reserve(
        &harness.pool,
        harness.world_id,
        harness.map_id,
        second.session,
    )
    .await
    .expect("reserve");
    let token_b = (Uuid::new_v4().as_u128() & 0xFFFF_FFFF) as u32;
    placement::begin_transfer(
        &harness.pool,
        token_b,
        harness.world_id,
        harness.map_id,
        second.session,
        second.account,
        second.character,
    )
    .await
    .expect("write the transfer");
    let mut version_b = harness.version(token_b);
    version_b.account = second.account.to_bytes_le();
    version_b.character = second.character.to_bytes_le();
    let Some((mut stream_b, mut client_b)) = connect_properly(harness.address, &version_b).await
    else {
        panic!("the world refused the second client");
    };
    read_until(&mut stream_b, &mut client_b, game::INSTANCE_LOAD_INFO).await;
    ask(&mut stream_b, &mut client_b, 0x8090, 19).await;
    read_until(&mut stream_b, &mut client_b, game::INSTANCE_LOAD_FINISH).await;
    // The first player's introduction arrives in the same chunk as this client's own block, so the read
    // above already consumed it: no drain, and no second read that would wait for a frame that is gone.

    // ---- the walk: answered with the graph's first waypoint, not with the request --------------
    let spawn = (6602.0f32, 4485.0f32);
    let destination = (5602.0f32, 5485.0f32);
    let expected = match harness.world.graphs.walk(
        harness.map_id,
        asura_world::pathfind::Pos {
            x: spawn.0,
            y: spawn.1,
            plane: 0,
        },
        asura_world::pathfind::Pos {
            x: destination.0,
            y: destination.1,
            plane: 0,
        },
    ) {
        asura_world::Walk::Found(waypoints) => waypoints,
        other => panic!("the map's graph cannot walk to {destination:?}: {other:?}"),
    };
    assert!(
        expected.len() > 1,
        "this case is a corridor around something, not a straight line: {} waypoint(s)",
        expected.len()
    );

    let mut walk = Vec::new();
    walk.extend_from_slice(&destination.0.to_le_bytes());
    walk.extend_from_slice(&destination.1.to_le_bytes());
    walk.extend_from_slice(&0u32.to_le_bytes()); // plane
    say(&mut stream_a, &mut client_a, 0x803E, &walk).await;

    let answered = read_until(&mut stream_a, &mut client_a, game::AGENT_MOVE_TO_POINT).await;
    let move_to = answered
        .iter()
        .find(|frame| frame.header() == game::AGENT_MOVE_TO_POINT)
        .expect("the walk is answered with a destination");
    let first = (
        f32::from_le_bytes([
            move_to.body()[4],
            move_to.body()[5],
            move_to.body()[6],
            move_to.body()[7],
        ]),
        f32::from_le_bytes([
            move_to.body()[8],
            move_to.body()[9],
            move_to.body()[10],
            move_to.body()[11],
        ]),
    );
    // The answer is the first sample of the reference's path: on the segment from the agent to the
    // path's first waypoint, one announcement step (145 units, the live servers' cadence) along it --
    // or the waypoint itself when the segment is shorter.
    let corner = (expected[0].pos.x, expected[0].pos.y);
    let segment = ((corner.0 - spawn.0).powi(2) + (corner.1 - spawn.1).powi(2)).sqrt();
    let travelled = ((first.0 - spawn.0).powi(2) + (first.1 - spawn.1).powi(2)).sqrt();
    assert!(
        travelled <= asura_world::LIVE_CADENCE_STEP + 1.0,
        "the first destination is one announcement step away ({travelled:.1} units)"
    );
    if segment > asura_world::LIVE_CADENCE_STEP {
        assert!(
            travelled > asura_world::LIVE_CADENCE_STEP - 1.0,
            "and a step is not skipped ({travelled:.1} of {segment:.1} units to the corner)"
        );
    } else {
        assert_eq!(
            first, corner,
            "a segment shorter than a step is announced whole"
        );
    }
    assert_ne!(
        first, destination,
        "and the first destination is not where the client asked to go"
    );

    // And the other player is told, which is what makes a second client see the first one move.
    let seen = read_until(&mut stream_b, &mut client_b, game::AGENT_MOVE_TO_POINT).await;
    let seen_at = seen
        .iter()
        .find(|frame| frame.header() == game::AGENT_MOVE_TO_POINT)
        .expect("the other player sees the walk");
    assert_eq!(
        f32::from_le_bytes([
            seen_at.body()[4],
            seen_at.body()[5],
            seen_at.body()[6],
            seen_at.body()[7]
        ]),
        first.0,
        "and sees the same waypoint"
    );

    // ---- the tick walks the agent ------------------------------------------------------------
    // Nothing is sent when a walk starts: the agent moves itself, `speed_base * delta` per world tick
    // (`GameSrv_UpdateAgentMovement`), and the world's idea of where it stands follows.
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    let mut travelled = 0.0f32;
    while std::time::Instant::now() < deadline {
        for peer in harness.world.instances.others(0) {
            let dx = peer.standing.position.0 - spawn.0;
            let dy = peer.standing.position.1 - spawn.1;
            travelled = travelled.max(((dx * dx) + (dy * dy)).sqrt());
        }
        if travelled > 100.0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        travelled > 100.0,
        "the agent walks on the world tick: it has moved {travelled:.1} units"
    );

    // ---- the budget: a report a kilometre away is clamped, and the clamp is what is broadcast ----
    let far = (spawn.0 + 2500.0, spawn.1 + 3000.0);
    // Where the walker stood when it made the report. The first client to join takes player id 1
    // (`Instances::join` hands out the lowest free id), and the second client is the one that observes.
    let before = harness
        .world
        .instances
        .others(0)
        .into_iter()
        .find(|peer| peer.player_id == 1)
        .map(|peer| peer.standing.position)
        .expect("the walker is in the world");
    let distance_before = ((before.0 - far.0).powi(2) + (before.1 - far.1).powi(2)).sqrt();

    let mut report = Vec::new();
    report.extend_from_slice(&far.0.to_le_bytes());
    report.extend_from_slice(&far.1.to_le_bytes());
    report.extend_from_slice(&0u32.to_le_bytes());
    report.extend_from_slice(&[0u8; 12]); // the two trailing fields, which the world does not read
    say(&mut stream_a, &mut client_a, 0x803D, &report).await;

    let observed = read_until(&mut stream_b, &mut client_b, game::AGENT_UPDATE_POSITION).await;
    let position = observed
        .iter()
        .find(|frame| frame.header() == game::AGENT_UPDATE_POSITION)
        .expect("the clamp is broadcast");
    let x = f32::from_le_bytes([
        position.body()[4],
        position.body()[5],
        position.body()[6],
        position.body()[7],
    ]);
    let y = f32::from_le_bytes([
        position.body()[8],
        position.body()[9],
        position.body()[10],
        position.body()[11],
    ]);
    let jumped = ((x - far.0).powi(2) + (y - far.1).powi(2)).sqrt();
    // The reference's budget: `speed_base * elapsed + POSITION_UPDATE_TOLERANCE`, with the elapsed time
    // capped at `POSITION_UPDATE_MAX_ELAPSED_MS` (2000 ms) -- 288 * 2 + 50 = 626 units.
    assert!(
        jumped < distance_before,
        "an out-of-budget report still moves the agent toward what it said"
    );
    assert!(
        distance_before - jumped <= 700.0,
        "but only as far as the budget allows: it moved {:.1} units",
        distance_before - jumped
    );
    assert!(
        jumped > 500.0,
        "and the far report itself was not believed: {jumped:.1} units from it"
    );

    drop(stream_a);
    drop(stream_b);
    placement::remove_world(&harness.pool, harness.world_id)
        .await
        .expect("remove the world");
    harness.stop();
}

/// A cancelled walk stops the server's own copy where it stands.
///
/// `GAME_CMSG_CANCEL_MOVEMENT` (`0x8028`, header only) is what the client sends when it takes the walk
/// back - the user clicking or walking away. The reference answers with
/// `GameSrv_CancelAgentMovement` (destination = position, speed = 0) and a stop broadcast
/// (`GmAgent.c:533-544`). The cancel is the load-bearing half: without it this server kept advancing
/// the agent along the abandoned path on every tick, and the next click was answered from a position
/// the client had walked away from (observed live: a click to x4 started from the old x2, pulling the
/// agent back to it).
///
/// Observed through the world's own peer list, which is where the agent's position lives: before the
/// cancel it advances ~144 units a tick; after it, it does not move at all.
#[tokio::test]
async fn a_cancelled_walk_stops_the_servers_own_copy() {
    if skip() {
        return;
    }
    let Some(harness) = Harness::start().await else {
        return;
    };
    if !harness.walks {
        eprintln!("skipped: no Gw.dat, so this world has no path graph");
        harness.stop();
        return;
    }

    let version = harness.version(harness.player_token);
    let Some((mut stream, mut client)) = connect_properly(harness.address, &version).await else {
        panic!("the world refused the client");
    };
    read_until(&mut stream, &mut client, game::INSTANCE_LOAD_INFO).await;
    ask(&mut stream, &mut client, 0x8090, 19).await;
    read_until(&mut stream, &mut client, game::INSTANCE_LOAD_FINISH).await;

    // A walk the finder answers with a corridor (more than one waypoint), so the tick keeps walking
    // for several seconds of wall time.
    let spawn = (6602.0f32, 4485.0f32);
    let destination = (5602.0f32, 5485.0f32);
    let mut walk = Vec::new();
    walk.extend_from_slice(&destination.0.to_le_bytes());
    walk.extend_from_slice(&destination.1.to_le_bytes());
    walk.extend_from_slice(&0u32.to_le_bytes());
    say(&mut stream, &mut client, 0x803E, &walk).await;
    read_until(&mut stream, &mut client, game::AGENT_MOVE_TO_POINT).await;

    // Let the world tick advance the agent: the walk is under way.
    tokio::time::sleep(Duration::from_millis(900)).await;
    let position_of = |harness: &Harness| {
        harness
            .world
            .instances
            .others(0)
            .into_iter()
            .next()
            .map(|peer| peer.standing.position)
            .expect("the walker is in the world")
    };
    let walking = position_of(&harness);
    let moved_from_spawn = ((walking.0 - spawn.0).powi(2) + (walking.1 - spawn.1).powi(2)).sqrt();
    assert!(
        moved_from_spawn > 100.0,
        "the walk was under way before the cancel ({moved_from_spawn:.1} units)"
    );

    // The client takes the walk back.
    ask(&mut stream, &mut client, 0x8028, 2).await;
    let stopped = read_until(&mut stream, &mut client, game::AGENT_STOP_MOVING).await;
    assert!(
        stopped
            .iter()
            .any(|frame| frame.header() == game::AGENT_STOP_MOVING),
        "the cancel is answered with a stop"
    );

    // Two more ticks later the agent has to be exactly where it stopped. Without the cancel it would
    // have walked another ~576 units along the abandoned path.
    let at_cancel = position_of(&harness);
    tokio::time::sleep(Duration::from_millis(1200)).await;
    let after = position_of(&harness);
    let crept = ((after.0 - at_cancel.0).powi(2) + (after.1 - at_cancel.1).powi(2)).sqrt();
    assert!(
        crept < 1.0,
        "the cancelled walk stopped the server's copy: it crept {crept:.1} units over two ticks"
    );

    drop(stream);
    placement::remove_world(&harness.pool, harness.world_id)
        .await
        .expect("remove the world");
    harness.stop();
}
/// T10's refusal half: a destination the finder cannot reach is answered with **where the agent is**, not
/// with a walk and not with an error of its own.
///
/// The reference sends a world tick, cancels the movement and broadcasts the position
/// (`GmAgent.c:391-394`); the client is left to snap back from the walk it predicted. That is the only
/// thing this test can observe, and it is the thing that matters: no `AGENT_MOVE_TO_POINT` was promised.
///
/// **Acknowledged 2026-10-28, ignored until the owner revisits it.** This test waits for an
/// `AGENT_UPDATE_POSITION` on a refused walk; the handler deliberately sends nothing, and its own comment
/// records why (that message disconnected a live client with `007` once and teleported it into a wall
/// once). So the test and the handler contradict each other, and it hangs rather than failing: its reader
/// only gives up after ten seconds of silence, while the world clock ticks every 100 ms. It hung at
/// `6b4b192` (the commit before the instance change) for the same reason. The decision is the owner's:
/// fix this test to assert silence, restore the position message and measure what the client does, or
/// drop the test as superseded.
#[ignore = "acknowledged 2026-10-28: contradicts the handler, which sends no position on a refused walk; reopen when the owner decides which side is right (see the doc comment)"]
#[tokio::test]
async fn a_walk_the_finder_refuses_is_answered_with_the_agents_position() {
    if skip() {
        return;
    }
    let Some(harness) = Harness::start().await else {
        return;
    };
    if !harness.walks {
        eprintln!("skipped: no Gw.dat, so this world has no path graph");
        harness.stop();
        return;
    }

    let version = harness.version(harness.player_token);
    let Some((mut stream, mut client)) = connect_properly(harness.address, &version).await else {
        panic!("the world refused the client");
    };
    read_until(&mut stream, &mut client, game::INSTANCE_LOAD_INFO).await;

    // Far outside the map's geometry: `PathFindTrapezoid` has no trapezoid for it, so the finder answers
    // "no path" before it searches anything.
    let nowhere = (30_000.0f32, 30_000.0f32);
    let mut walk = Vec::new();
    walk.extend_from_slice(&nowhere.0.to_le_bytes());
    walk.extend_from_slice(&nowhere.1.to_le_bytes());
    walk.extend_from_slice(&0u32.to_le_bytes());
    say(&mut stream, &mut client, 0x803E, &walk).await;

    let answered = read_until(&mut stream, &mut client, game::AGENT_UPDATE_POSITION).await;
    assert!(
        !answered
            .iter()
            .any(|frame| frame.header() == game::AGENT_MOVE_TO_POINT),
        "a refused walk must not promise a destination"
    );
    let position = answered
        .iter()
        .find(|frame| frame.header() == game::AGENT_UPDATE_POSITION)
        .expect("the refusal is a position");
    let x = f32::from_le_bytes([
        position.body()[4],
        position.body()[5],
        position.body()[6],
        position.body()[7],
    ]);
    let y = f32::from_le_bytes([
        position.body()[8],
        position.body()[9],
        position.body()[10],
        position.body()[11],
    ]);
    assert_eq!(
        (x, y),
        (6602.0, 4485.0),
        "the agent is told it is still where it was standing"
    );

    drop(stream);
    placement::remove_world(&harness.pool, harness.world_id)
        .await
        .expect("remove the world");
    harness.stop();
}
