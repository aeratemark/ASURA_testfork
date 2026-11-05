//! The whole chain, in one test: login, travel, and a character in a map.
//!
//! This is the milestone's acceptance test without the game client. Both services run in this process,
//! a client logs in to the auth service, asks to travel, is told where to go, and connects to the world
//! server with the tokens it was given. What it asserts, in order:
//!
//! 1. the login batch arrives and the connection is logged in;
//! 2. the travel request is answered with `GAME_SERVER_INFO` and a success response -'- not a refusal;
//! 3. the address in that message is where the client actually gets in;
//! 4. the tokens in that message are the ones the world server believes;
//! 5. the client reaches `INSTANCE_LOAD_FINISH`, the message that ends the loading screen;
//! 6. the slot the travel took is released when the player leaves.
//!
//! Step 3 is the one nothing else covers: the auth service writes an address into a 24-byte blob in the
//! reference's own encoding, and the client has to reconstruct a working socket address from it.
//!
//! With no database configured, the test prints a skip line and returns.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use asura_auth::Trace;
use asura_db::login as db;
use asura_db::{placement, PgPool, Uuid};
use asura_gateway::serve as serve_auth;
use asura_protocol::client::game_version::GameVersion;
use asura_protocol::client::handshake;
use asura_protocol::client::messages::{self, RequestResponse};
use asura_protocol::client::reference_client::ReferenceClient;
use asura_protocol::client::{tables, Frame};
use asura_protocol::game;
use asura_world::{serve as serve_world, MapEntry, World};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const TIMEOUT: Duration = Duration::from_secs(10);
const LOGIN_REQUEST: u32 = 0x1111;
const TRAVEL_REQUEST: u32 = 0x2222;

const MAP_ID: i32 = 146;
const MAP_BLOB: u32 = 0x1B97D;
const SPAWN: (f32, f32) = (6602.0, 4485.0);

fn skip() -> bool {
    if asura_db::config::database_url().is_none() {
        println!("ASURA_DATABASE_URL not set; skipping the end-to-end travel-and-load test");
        return true;
    }
    false
}

fn unique_world() -> i32 {
    30_000 + (Uuid::new_v4().as_u128() % 10_000) as i32
}

async fn read_exact(stream: &mut TcpStream, count: usize) -> Vec<u8> {
    let mut bytes = vec![0u8; count];
    tokio::time::timeout(TIMEOUT, stream.read_exact(&mut bytes))
        .await
        .expect("read before the timeout")
        .expect("read");
    bytes
}

/// Version message and key exchange on the auth connection.
async fn auth_handshake(stream: &mut TcpStream) -> ReferenceClient {
    let mut client = ReferenceClient::with_seed(0xA11CE);
    stream
        .write_all(&client.version_message(34591))
        .await
        .expect("write the version message");
    stream
        .write_all(&client.client_seed_message())
        .await
        .expect("write the client seed");
    let server_seed = read_exact(stream, handshake::SERVER_SEED_LEN).await;
    client
        .accept_server_seed(&server_seed)
        .expect("server seed");
    client
}

/// Read auth frames until `done` says so.
async fn read_auth_frames(
    stream: &mut TcpStream,
    client: &mut ReferenceClient,
    done: impl Fn(&Frame) -> bool,
) -> Vec<Frame> {
    let mut frames: Vec<Frame> = Vec::new();
    let mut buffer = [0u8; 8192];
    while !frames.iter().any(&done) {
        let read = tokio::time::timeout(TIMEOUT, stream.read(&mut buffer))
            .await
            .expect("read before the timeout")
            .expect("read");
        assert!(read > 0, "the auth service closed the connection");
        frames.extend(client.receive(&buffer[..read]).expect("frames"));
    }
    frames
}

#[tokio::test]
async fn a_client_logs_in_travels_and_reaches_its_map() {
    if skip() {
        return;
    }
    let url = asura_db::config::database_url().expect("a database URL");
    let pool: PgPool = placement::connect(&url, 6).await.expect("connect");

    // ---- the two services --------------------------------------------------------------------
    let world_id = unique_world();
    let world_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind the world port");
    let world_address = world_listener.local_addr().expect("world address");
    let world = Arc::new(World {
        pool: pool.clone(),
        world_id,
        maps: vec![MapEntry {
            map_id: MAP_ID,
            map_file_id: MAP_BLOB,
            map_type: 0,
            spawn: SPAWN,
            spawn_plane: 0,
            spawns: Vec::new(),
            name: "Lakeside County".to_string(),
            capacity: 2,
        }],
        replay: None,
        instances: Default::default(),
        public: world_address,
        // This test is the login chain, not movement: no graph, so every walk would be refused.
        graphs: asura_world::PathGraphs::empty(),
        lazy_maps: Default::default(),
        load_on_demand: false,
        capacity: 2,
        archive: String::new(),
    });
    let world_task = tokio::spawn(async move {
        let _ = serve_world(world_listener, world, true).await;
    });

    // The world registers itself the way `asura-world`'s startup does: the address a client may dial.
    placement::register_world(
        &pool,
        world_id,
        "e2e-stack",
        &world_address.ip().to_string(),
        i32::from(world_address.port()),
        "m4-test",
        &[(MAP_ID, 2)],
    )
    .await
    .expect("register the world");

    let auth_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind the auth port");
    let auth_address = auth_listener.local_addr().expect("auth address");
    let auth_pool = pool.clone();
    let auth_task = tokio::spawn(async move {
        let _ = serve_auth(auth_listener, auth_pool, Trace::from_environment()).await;
    });

    // ---- a client that logs in ---------------------------------------------------------------
    let seeded = db::seed_identity(
        &pool,
        Uuid::new_v4(),
        Uuid::new_v4(),
        &format!("m4-stack-{}", Uuid::new_v4()),
        "Asura Stack",
        MAP_ID,
    )
    .await
    .expect("seed the identity");

    let mut auth_stream = tokio::time::timeout(TIMEOUT, TcpStream::connect(auth_address))
        .await
        .expect("connect before the timeout")
        .expect("connect to auth");
    let mut auth_client = auth_handshake(&mut auth_stream).await;

    let login = auth_client
        .login_message(
            LOGIN_REQUEST,
            seeded.portal_user.to_bytes_le(),
            seeded.session.to_bytes_le(),
        )
        .expect("build the login");
    let mut bytes = login.encode();
    auth_client.seal_raw(&mut bytes);
    auth_stream
        .write_all(&bytes)
        .await
        .expect("write the login");
    let frames = read_auth_frames(&mut auth_stream, &mut auth_client, |frame| {
        frame.header() == tables::AUTH_SMSG_REQUEST_RESPONSE
            && RequestResponse::decode(frame).is_ok_and(|reply| reply.request == LOGIN_REQUEST)
    })
    .await;
    assert_eq!(
        RequestResponse::decode(frames.last().expect("a response"))
            .expect("decodes")
            .status,
        messages::STATUS_OK,
        "the login batch ends with success"
    );

    // ---- the travel request ------------------------------------------------------------------
    let mut body = Vec::new();
    for value in [TRAVEL_REQUEST, 0u32, MAP_ID as u32, 0, 0, 0] {
        body.extend_from_slice(&value.to_le_bytes());
    }
    let mut bytes = Frame::new(tables::AUTH_CMSG_REQUEST_GAME_INSTANCE, body).encode();
    auth_client.seal_raw(&mut bytes);
    auth_stream
        .write_all(&bytes)
        .await
        .expect("write the travel request");
    let frames = read_auth_frames(&mut auth_stream, &mut auth_client, |frame| {
        frame.header() == tables::AUTH_SMSG_REQUEST_RESPONSE
            && RequestResponse::decode(frame).is_ok_and(|reply| reply.request == TRAVEL_REQUEST)
    })
    .await;

    let info = frames
        .iter()
        .find(|frame| frame.header() == tables::AUTH_SMSG_GAME_SERVER_INFO)
        .expect("a travel that is placed is answered with GAME_SERVER_INFO");
    assert_eq!(info.len(), 42);

    let reply = RequestResponse::decode(frames.last().expect("a response")).expect("decodes");
    assert_eq!(reply.status, messages::STATUS_OK, "and with success");

    // ---- the address, read the way a client reads it -----------------------------------------
    //
    // The client is told to dial the **auth** address: that is what a real client does (measured -'- it
    // ignored the port in this same message and dialled the address it was launched with), and it is
    // what the reference does, whose auth port receives game connections and hands them to a game
    // server. This service routes the connection onwards (`asura_gateway::relay`).
    let host = &info.body()[12..36];
    assert_eq!(&host[0..2], &[2, 0], "AF_INET");
    let port = u16::from_be_bytes([host[2], host[3]]);
    let ip = std::net::Ipv4Addr::new(host[4], host[5], host[6], host[7]);
    let game_address = SocketAddr::from((ip, port));
    assert_eq!(
        game_address, auth_address,
        "the client is sent to the auth service, which routes the game connection onwards"
    );
    assert_ne!(
        auth_address, world_address,
        "and that is a different socket from the world's own (the routing is not accidental)"
    );

    let map_token = u32::from_le_bytes([
        info.body()[4],
        info.body()[5],
        info.body()[6],
        info.body()[7],
    ]);
    // req_id(4) + map_token(4) + map_id(4) + host(24) = 36, then the player token.
    let player_token = u32::from_le_bytes([
        info.body()[36],
        info.body()[37],
        info.body()[38],
        info.body()[39],
    ]);
    assert_eq!(map_token, world_id as u32, "the map token is the world id");
    assert_ne!(player_token, 0, "and a player token was issued");

    // ---- the world connection ----------------------------------------------------------------
    let version = GameVersion {
        version: 39112,
        h0008: 1,
        map_token,
        map_id: MAP_ID as u32,
        player_token,
        account: seeded.account.to_bytes_le(),
        character: seeded.character.to_bytes_le(),
        h0038: 0,
        h003c: 0,
    };
    let mut game_stream = tokio::time::timeout(TIMEOUT, TcpStream::connect(game_address))
        .await
        .expect("connect before the timeout")
        .expect("connect to the world");
    game_stream
        .write_all(&version.encode())
        .await
        .expect("write the game version");
    let mut game_client = ReferenceClient::with_seed(0xB0B);
    game_stream
        .write_all(&game_client.client_seed_message())
        .await
        .expect("write the client seed");
    let server_seed = read_exact(&mut game_stream, handshake::SERVER_SEED_LEN).await;
    game_client
        .accept_server_seed(&server_seed)
        .expect("server seed");

    // The world sends its load block unasked; ask for the three things that finish it.
    let mut frames: Vec<Frame> = Vec::new();
    let mut buffer = [0u8; 8192];

    // (a) the load block
    while !frames
        .iter()
        .any(|frame| frame.header() == game::INSTANCE_LOAD_INFO)
    {
        let read = tokio::time::timeout(TIMEOUT, game_stream.read(&mut buffer))
            .await
            .expect("read before the timeout")
            .expect("read");
        assert!(
            read > 0,
            "the world closed the connection during the load block"
        );
        frames.extend(game_client.receive_game(&buffer[..read]).expect("frames"));
    }

    for (header, size) in [(0x8091u16, 4usize), (0x8088, 2), (0x8090, 19)] {
        let mut request = vec![0u8; size];
        request[0..2].copy_from_slice(&header.to_le_bytes());
        game_client.seal_raw(&mut request);
        game_stream
            .write_all(&request)
            .await
            .expect("write a load request");
    }

    let mut saw_finish = frames
        .iter()
        .any(|frame| frame.header() == game::INSTANCE_LOAD_FINISH);
    while !saw_finish {
        let read = tokio::time::timeout(TIMEOUT, game_stream.read(&mut buffer))
            .await
            .expect("read before the timeout")
            .expect("read");
        assert!(read > 0, "the world closed the connection during the load");
        let arrived = game_client.receive_game(&buffer[..read]).expect("frames");
        saw_finish = arrived
            .iter()
            .any(|frame| frame.header() == game::INSTANCE_LOAD_FINISH);
        frames.extend(arrived);
    }

    // The spawn point names the blob and the coordinate the map's config holds.
    let spawn = frames
        .iter()
        .find(|frame| frame.header() == game::INSTANCE_LOAD_SPAWN_POINT)
        .expect("a spawn point");
    assert_eq!(
        u32::from_le_bytes([
            spawn.body()[0],
            spawn.body()[1],
            spawn.body()[2],
            spawn.body()[3]
        ]),
        MAP_BLOB
    );
    assert_eq!(
        f32::from_le_bytes([
            spawn.body()[4],
            spawn.body()[5],
            spawn.body()[6],
            spawn.body()[7]
        ]),
        SPAWN.0
    );

    // ---- leaving releases the slot -----------------------------------------------------------
    drop(game_stream);
    let mut released = false;
    for _ in 0..60 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        if placement::pool_counts(&pool, world_id, MAP_ID)
            .await
            .expect("read the counter")
            == Some((2, 0))
        {
            released = true;
            break;
        }
    }
    assert!(released, "the travel's slot went back to the pool");

    drop(auth_stream);
    placement::remove_world(&pool, world_id)
        .await
        .expect("remove the world");
    auth_task.abort();
    world_task.abort();
}
