//! M4's end-to-end test: a client that has logged in asks to travel, and the auth service answers
//! with a world it can reach.
//!
//! What is proved here, in the order the assertions appear:
//!
//! * a travel request to a map a fresh world publishes is answered with `AUTH_SMSG_GAME_SERVER_INFO`
//!   (42 bytes) and then the success response - the reference's pair, in that order;
//! * the info message names the world's address and port in the reference's own encoding, the map the
//!   client asked for, and a player token that is not the same every time;
//! * the travel *took a lease*, so a second client cannot be given more slots than the world published;
//! * a map nobody publishes is refused with status 2 and no info message;
//! * a world that is registered but not listening is refused too - the one rule: never send a client
//!   to a port nothing answers on - and the refusal leaves the counter untouched.
//!
//! With no database configured, each test prints a skip line and returns.

use std::net::SocketAddr;
use std::time::Duration;

use asura_auth::{serve, Trace};
use asura_db::login as db;
use asura_db::{placement, PgPool, Uuid};
use asura_protocol::client::handshake;
use asura_protocol::client::messages::{self, RequestResponse};
use asura_protocol::client::peer::ReferenceClient;
use asura_protocol::client::{tables, Frame};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const TIMEOUT: Duration = Duration::from_secs(10);
const TRAVEL_REQUEST: u32 = 0x2A2A;

/// The auth service on a loopback port, and the pool it uses.
struct Harness {
    pool: PgPool,
    address: SocketAddr,
    server: tokio::task::JoinHandle<()>,
}

impl Harness {
    async fn start() -> Option<Harness> {
        let url = asura_db::config::database_url()?;
        let pool = placement::connect(&url, 4)
            .await
            .expect("connect the database");

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind a loopback port");
        let address = listener.local_addr().expect("local address");
        let server_pool = pool.clone();
        let server = tokio::spawn(async move {
            let _ = serve(listener, server_pool, Trace::from_environment()).await;
        });

        Some(Harness {
            pool,
            address,
            server,
        })
    }

    fn stop(self) {
        self.server.abort();
    }
}

fn skip() -> bool {
    if asura_db::config::database_url().is_none() {
        println!("ASURA_DATABASE_URL not set; skipping the end-to-end travel test");
        return true;
    }
    false
}

/// A map id no other test uses: travel picks the least-loaded world that publishes a map, so sharing
/// one would make two tests race for the same slot.
fn unique_map() -> i32 {
    40_000 + (Uuid::new_v4().as_u128() % 10_000) as i32
}

/// A world id no other test uses, in a range the services never pick by default.
fn unique_world() -> i32 {
    10_000 + (Uuid::new_v4().as_u128() % 10_000) as i32
}

/// A listening socket that accepts and immediately drops, standing in for a world server's port.
///
/// Drops are enough for a probe: what the auth service asks a world is "is someone accepting here",
/// and answering that honestly is the point. The real handshake is `asura-world`'s job.
async fn a_world_port() -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind a world port");
    let address = listener.local_addr().expect("local address");
    let task = tokio::spawn(async move {
        loop {
            if listener.accept().await.is_err() {
                break;
            }
        }
    });
    (address, task)
}

async fn read_exact(stream: &mut TcpStream, count: usize) -> Vec<u8> {
    let mut bytes = vec![0u8; count];
    tokio::time::timeout(TIMEOUT, stream.read_exact(&mut bytes))
        .await
        .expect("read before the timeout")
        .expect("read");
    bytes
}

/// Version message and key exchange, leaving the connection ready to log in.
async fn handshake_with(stream: &mut TcpStream) -> ReferenceClient {
    let mut client = ReferenceClient::with_seed(0xBADF00D);
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
        .expect("the server seed is well formed");
    client
}

/// Log in and read until the login batch's response, so the connection is in the logged-in state.
async fn log_in(
    stream: &mut TcpStream,
    client: &mut ReferenceClient,
    portal_user: Uuid,
    session: Uuid,
) {
    let frame = client
        .login_message(0x1, portal_user.to_bytes_le(), session.to_bytes_le())
        .expect("build the login message");
    let mut bytes = frame.encode();
    client.seal_raw(&mut bytes);
    stream.write_all(&bytes).await.expect("write the login");

    let mut buffer = [0u8; 8192];
    let mut plaintext: Vec<Frame> = Vec::new();
    while !plaintext
        .iter()
        .any(|frame| frame.header() == tables::AUTH_SMSG_REQUEST_RESPONSE)
    {
        let read = tokio::time::timeout(TIMEOUT, stream.read(&mut buffer))
            .await
            .expect("read before the timeout")
            .expect("read");
        assert!(read > 0, "the server closed the connection during login");
        plaintext.extend(client.receive(&buffer[..read]).expect("frames"));
    }
}

/// Ask to travel to `map_id`, and collect everything the server answers for it.
async fn travel(stream: &mut TcpStream, client: &mut ReferenceClient, map_id: u32) -> Vec<Frame> {
    let mut body = Vec::new();
    for value in [TRAVEL_REQUEST, 0, map_id, 0, 0, 0] {
        body.extend_from_slice(&value.to_le_bytes());
    }
    let mut bytes = Frame::new(tables::AUTH_CMSG_REQUEST_GAME_INSTANCE, body).encode();
    assert_eq!(bytes.len(), 26, "the request is fixed size");
    client.seal_raw(&mut bytes);
    stream
        .write_all(&bytes)
        .await
        .expect("write the travel request");

    // The answer is one or two frames; read until the response for this request shows up.
    let mut frames: Vec<Frame> = Vec::new();
    let mut buffer = [0u8; 8192];
    while !frames.iter().any(|frame| {
        frame.header() == tables::AUTH_SMSG_REQUEST_RESPONSE
            && RequestResponse::decode(frame).is_ok_and(|reply| reply.request == TRAVEL_REQUEST)
    }) {
        let read = tokio::time::timeout(TIMEOUT, stream.read(&mut buffer))
            .await
            .expect("read before the timeout")
            .expect("read");
        assert!(read > 0, "the server closed the connection during travel");
        frames.extend(client.receive(&buffer[..read]).expect("frames"));
    }
    frames
}

#[tokio::test]
async fn a_travel_request_is_answered_with_a_reachable_world() {
    if skip() {
        return;
    }
    let Some(harness) = Harness::start().await else {
        return;
    };

    let map_id = unique_map();
    let world_id = unique_world();
    let (world_address, world_task) = a_world_port().await;
    placement::register_world(
        &harness.pool,
        world_id,
        "e2e-world",
        &world_address.ip().to_string(),
        world_address.port() as i32,
        "m4-test",
        &[(map_id, 2)],
    )
    .await
    .expect("register the world");

    let seeded = db::seed_identity(
        &harness.pool,
        Uuid::new_v4(),
        Uuid::new_v4(),
        &format!("m4-e2e-{}", Uuid::new_v4()),
        "Asura Travel",
        map_id,
    )
    .await
    .expect("seed the identity");

    let mut stream = tokio::time::timeout(TIMEOUT, TcpStream::connect(harness.address))
        .await
        .expect("connect before the timeout")
        .expect("connect");
    let mut client = handshake_with(&mut stream).await;
    log_in(&mut stream, &mut client, seeded.portal_user, seeded.session).await;
    let frames = travel(&mut stream, &mut client, map_id as u32).await;

    let headers: Vec<u16> = frames.iter().map(|frame| frame.header()).collect();
    assert_eq!(
        headers,
        vec![
            tables::AUTH_SMSG_GAME_SERVER_INFO,
            tables::AUTH_SMSG_REQUEST_RESPONSE
        ],
        "the info message, then the response -- the reference's pair, in that order"
    );

    let info = &frames[0];
    assert_eq!(info.len(), 42, "the info message is 42 bytes");
    assert_eq!(info.body().len(), 40);

    // req_id, map_token, map_id, host[24], player_token.
    let dword = |offset: usize| {
        u32::from_le_bytes([
            info.body()[offset],
            info.body()[offset + 1],
            info.body()[offset + 2],
            info.body()[offset + 3],
        ])
    };
    assert_eq!(dword(0), TRAVEL_REQUEST, "the request id is echoed");
    assert_eq!(dword(4), world_id as u32, "the map token is the world id");
    assert_eq!(dword(8), map_id as u32, "the map is the one asked for");
    assert_ne!(dword(36), 0, "a player token was issued");

    // The host blob: AF_INET little endian, port big endian, then the address bytes. It names the
    // **auth** address, because that is where a client dials its game connection (measured, not
    // assumed) and where this service routes it onwards.
    assert_eq!(&info.body()[12..14], &[0x02, 0x00], "AF_INET, little endian");
    assert_eq!(
        &info.body()[14..16],
        &(harness.address.port()).to_be_bytes(),
        "the port is big endian, and it is the auth port"
    );
    let advertised = match harness.address.ip() {
        std::net::IpAddr::V4(v4) => v4.octets(),
        std::net::IpAddr::V6(_) => panic!("the test listener binds IPv4"),
    };
    assert_eq!(&info.body()[16..20], &advertised, "and the address itself");

    let reply = RequestResponse::decode(&frames[1]).expect("decodes");
    assert_eq!(
        reply.status,
        messages::STATUS_OK,
        "and the travel succeeded"
    );

    // The slot was taken, not just reported: a second travel can still fit (capacity 2), a third
    // cannot. This is the assertion that makes the answer mean something.
    let counts = placement::pool_counts(&harness.pool, world_id, map_id)
        .await
        .expect("read the counter");
    assert_eq!(counts, Some((2, 1)), "one slot is leased by the travel");

    // And the world can now believe that token: the pending transfer names this world, this map, this
    // account and this character. A token nobody claims is worthless. A second claim by the same world
    // is the client's newer connection, which rule 1 says to honour (`docs/05-gw2-lessons.md`), so it
    // returns the same transfer instead of nothing.
    let player_token = dword(36);
    let transfer = placement::claim_transfer(&harness.pool, world_id, player_token)
        .await
        .expect("claim the transfer")
        .expect("the transfer the travel just issued");
    assert_eq!(transfer.map_id, map_id);
    assert_eq!(transfer.session_id, seeded.session);
    assert_eq!(transfer.account_id, seeded.account);
    assert_eq!(transfer.character_id, seeded.character);
    assert_eq!(
        placement::claim_transfer(&harness.pool, world_id, player_token)
            .await
            .expect("claim again")
            .expect("the newer connection is the one to honour"),
        transfer,
        "a second connection with the same token is the newer one, not a replay to refuse"
    );
    assert_eq!(
        placement::claim_transfer(&harness.pool, world_id + 1, player_token)
            .await
            .expect("claim at another world"),
        None,
        "and not by a world it was not issued for"
    );

    let freed = placement::release(&harness.pool, seeded.session)
        .await
        .expect("release");
    assert!(freed, "the session's lease is the one that was taken");

    placement::remove_world(&harness.pool, world_id)
        .await
        .expect("remove the world");
    world_task.abort();
    harness.stop();
}

#[tokio::test]
async fn a_map_nobody_publishes_is_refused_without_an_info_message() {
    if skip() {
        return;
    }
    let Some(harness) = Harness::start().await else {
        return;
    };

    let map_id = unique_map();
    let seeded = db::seed_identity(
        &harness.pool,
        Uuid::new_v4(),
        Uuid::new_v4(),
        &format!("m4-refuse-{}", Uuid::new_v4()),
        "Asura Refused",
        map_id,
    )
    .await
    .expect("seed the identity");

    let mut stream = tokio::time::timeout(TIMEOUT, TcpStream::connect(harness.address))
        .await
        .expect("connect before the timeout")
        .expect("connect");
    let mut client = handshake_with(&mut stream).await;
    log_in(&mut stream, &mut client, seeded.portal_user, seeded.session).await;
    let frames = travel(&mut stream, &mut client, map_id as u32).await;

    assert_eq!(
        frames.len(),
        1,
        "only the response, with no info message to act on"
    );
    assert_eq!(frames[0].header(), tables::AUTH_SMSG_REQUEST_RESPONSE);
    assert_eq!(
        RequestResponse::decode(&frames[0]).expect("decodes").status,
        asura_auth::travel::STATUS_NETWORK_ERROR,
        "GM_ERROR_NETWORK_ERROR, which is what the client can show"
    );

    harness.stop();
}

#[tokio::test]
async fn a_registered_world_that_is_not_listening_is_refused_and_takes_no_lease() {
    if skip() {
        return;
    }
    let Some(harness) = Harness::start().await else {
        return;
    };

    let map_id = unique_map();
    let world_id = unique_world();
    // A port that was bound and then closed: registered in the database, dead on the socket.
    let (dead_address, dead_task) = a_world_port().await;
    dead_task.abort();
    tokio::time::sleep(Duration::from_millis(50)).await;

    placement::register_world(
        &harness.pool,
        world_id,
        "e2e-dead-world",
        &dead_address.ip().to_string(),
        dead_address.port() as i32,
        "m4-test",
        &[(map_id, 4)],
    )
    .await
    .expect("register the world");

    let seeded = db::seed_identity(
        &harness.pool,
        Uuid::new_v4(),
        Uuid::new_v4(),
        &format!("m4-dead-{}", Uuid::new_v4()),
        "Asura Dead World",
        map_id,
    )
    .await
    .expect("seed the identity");

    let mut stream = tokio::time::timeout(TIMEOUT, TcpStream::connect(harness.address))
        .await
        .expect("connect before the timeout")
        .expect("connect");
    let mut client = handshake_with(&mut stream).await;
    log_in(&mut stream, &mut client, seeded.portal_user, seeded.session).await;
    let frames = travel(&mut stream, &mut client, map_id as u32).await;

    assert_eq!(frames.len(), 1, "a dead world must not be offered");
    assert_eq!(
        RequestResponse::decode(&frames[0]).expect("decodes").status,
        asura_auth::travel::STATUS_NETWORK_ERROR
    );

    // The refusal must not have taken a slot: the counter and the ledger are untouched.
    let counts = placement::pool_counts(&harness.pool, world_id, map_id)
        .await
        .expect("read the counter");
    assert_eq!(counts, Some((4, 0)), "the refusal took no slot");

    placement::remove_world(&harness.pool, world_id)
        .await
        .expect("remove the world");
    harness.stop();
}
