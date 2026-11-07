//! The auth service: the program the game talks to when it logs in.
//!
//! Owns: the login endpoint (key exchange, login, character list), sessions, and telling the client
//! which world to join. Does NOT own placement, character state, instances, or the gameplay
//! connection - those belong to the world servers (`docs/01-scope.md`).
//!
//! AzerothCore parallel: their `authserver` - realmlist, session key, and nothing else.
//!
//! The connection follows the order `docs/03-protocol.md` records, and each step is a module in
//! `asura-protocol`:
//!
//! ```text
//! 1. first four bytes      client::version      what kind of connection is this
//! 2. the version message   client::version      plaintext, 16 bytes
//! 3. the key exchange      client::handshake    two plaintext seeds, then everything is scrambled
//! 4. frames                client::frame        header plus a body the header's size fixes
//! 5. login                 client::messages + asura-db::login
//! ```
//!
//! Two behaviours worth stating, because they are rules from the reference and not preferences:
//!
//! * **A connection that is not understood is closed, not guessed at.** GWPS logs the bytes and drops
//!   the socket; `VersionError::Unknown` does the same here.
//! * **A login that does not resolve is answered with the client's auth error**, in a request
//!   response, rather than by silence or by a drop: the client shows an error it can describe, and
//!   the session stays closed (`asura_db::login::find_session` returns `None`).

use std::io;
use std::net::SocketAddr;
use std::time::Duration;

use asura_db::account as db_account;
use asura_db::game as db_game;
use asura_db::login as db_login;
use asura_db::PgPool;
use asura_protocol::client::handshake::{self, CLIENT_SEED_LEN, MASTER_SECRET_LEN};
use asura_protocol::client::messages::{self, PortalAccountLogin, RequestResponse, STATUS_OK};
use asura_protocol::client::tables;
use asura_protocol::client::version::{self, VersionError, AUTH_CMSG_VERSION_LEN};
use asura_protocol::client::{Frame, FrameError};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use uuid::Uuid;

/// Answering `AUTH_CMSG_REQUEST_GAME_INSTANCE`: which world, and whether there is one at all.
pub mod travel;

/// How long a connection may sit without sending anything before it is dropped.
///
/// Not from the reference - GWPS relies on the operating system's timeouts - but a login connection
/// that stalls holds a task and a pool connection, and losing it is free: the client reconnects.
pub const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(120);

/// How often a quiet connection is sent a heartbeat.
///
/// The reference sends AUTH_SMSG_HEARTBEAT on a connection that has gone quiet, and a real client
/// waits for it after the login batch. The exact interval is not recorded anywhere readable, so this is
/// five seconds: comfortably inside any client's patience, and cheap.
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);

/// The idle timeout in force, from `ASURA_IDLE_TIMEOUT_SECS` when it is set.
///
/// It is adjustable because the timeout is a *policy*, and the two policies disagree: a production
/// service wants stalled connections gone in a couple of minutes, while a person logging a client in
/// by hand may sit at the login screen for far longer than that with a perfectly healthy connection
/// open. Dropping that connection looks exactly like a protocol failure, which is the last thing a
/// debugging session needs.
pub fn idle_timeout() -> Duration {
    static TIMEOUT: std::sync::OnceLock<Duration> = std::sync::OnceLock::new();
    *TIMEOUT.get_or_init(|| {
        std::env::var("ASURA_IDLE_TIMEOUT_SECS")
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .filter(|seconds| *seconds > 0)
            .map(Duration::from_secs)
            .unwrap_or(DEFAULT_IDLE_TIMEOUT)
    })
}

/// How many bytes to read from the socket at once.
const READ_CHUNK: usize = 4096;

/// The most bytes a login's settings answer may carry: the declared capacity of
/// `AUTH_SMSG_ACCOUNT_SETTINGS`'s `Array8` (`crates/asura-protocol/src/packets.rs`, `0x0016`, 2048).
///
/// A settings upload larger than this cannot be answered in one login message, so it is refused
/// rather than stored and truncated on the way out.
const ACCOUNT_SETTINGS_CAPACITY: usize = 2048;

/// Why a connection ended.
#[derive(Debug)]
pub enum ConnectionError {
    /// The socket failed.
    Io(io::Error),
    /// The first message was not a version message.
    BadVersion(VersionError),
    /// The key exchange did not complete.
    Handshake(handshake::HandshakeError),
    /// A frame was malformed.
    Framing(FrameError),
    /// A message's body was malformed.
    Message(messages::MessageError),
    /// The connection was idle for too long.
    Idle,
    /// The database failed.
    Database(asura_db::DatabaseError),
}

impl core::fmt::Display for ConnectionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ConnectionError::Io(err) => write!(f, "socket: {err}"),
            ConnectionError::BadVersion(err) => write!(f, "version: {err}"),
            ConnectionError::Handshake(err) => write!(f, "key exchange: {err}"),
            ConnectionError::Framing(err) => write!(f, "framing: {err}"),
            ConnectionError::Message(err) => write!(f, "message: {err}"),
            ConnectionError::Idle => write!(f, "connection was idle"),
            ConnectionError::Database(err) => write!(f, "database: {err}"),
        }
    }
}

impl std::error::Error for ConnectionError {}

/// The settings upload the client sends after entering a map: a size announcement followed by one or
/// more content messages that share its request id (`logs/auth.log`, requests 5 and 10).
struct SettingsUpload {
    /// The request id the size message carried; only content with this id belongs to the upload.
    request: u32,
    /// The byte count the size message announced.
    total: u32,
    /// The bytes received so far.
    bytes: Vec<u8>,
}

/// One connection's state, after the key exchange.
struct Session {
    /// The connection's two scramblers, set up by the key exchange.
    established: handshake::Established,
    /// Decrypted bytes not yet consumed as frames.
    plaintext: Vec<u8>,
    /// The account this connection resolved to, once the identity pair was accepted.
    account: Option<Uuid>,
    /// The session row the pair named, kept so a character selection can be recorded on it and a
    /// travel can lease against it.
    session_id: Option<Uuid>,
    /// The character this connection last selected, if any: the one a travel puts in the map, and the
    /// one the account info names. The reference keeps the same value per connection
    /// (`conn->account.char_id`).
    character: Option<Uuid>,
    /// The salt sent in `AUTH_SMSG_SESSION_INFO`, kept for the trace and for later messages that
    /// sign against it. The reference keeps it per connection for the same reason.
    server_salt: u32,
    /// The account settings upload in flight, if the client is in the middle of one.
    settings_upload: Option<SettingsUpload>,
}

/// Whether to print the per-message trace.
///
/// Defined in `asura-protocol` because a world server reads the same switch for the same reason: the
/// client reports nothing but `007`, so the trace is the only way to see what actually differed
/// (`docs/03-protocol.md`, section 6).
pub use asura_protocol::Trace;

/// Accept connections until the listener fails.
///
/// Each connection is handled in its own task; a failure closes that connection and nothing else.
/// The service is restartable by design: it holds no state that a restart has to recover
/// (`docs/02-architecture.md`, "Startup order").
pub async fn converse(
    mut stream: TcpStream,
    mut buffer: Vec<u8>,
    pool: &PgPool,
    trace: Trace,
    advertised: SocketAddr,
) -> Result<(), ConnectionError> {
    // The four classification bytes are already in `buffer` and are the beginning of the version
    // message: the socket layer read them, decided this is a login connection, and handed the socket
    // over (`asura-gateway`, `docs/38-layer-decisions.md`, D26). The file check was answered there too,
    // because it carries no identity.

    // ---- 2. the version message -------------------------------------------------------------
    read_until(&mut stream, &mut buffer, AUTH_CMSG_VERSION_LEN).await?;
    let (client_version, used) =
        version::AuthVersion::decode(&buffer).map_err(ConnectionError::BadVersion)?;
    buffer.drain(..used);
    if trace.on() {
        eprintln!(
            "auth <- version build={} h0008={} h000c={}",
            client_version.version, client_version.h0008, client_version.h000c
        );
    }

    // ---- 3. the key exchange ---------------------------------------------------------------
    read_until(&mut stream, &mut buffer, CLIENT_SEED_LEN).await?;
    let mut master_secret = [0u8; MASTER_SECRET_LEN];
    getrandom::getrandom(&mut master_secret)
        .map_err(|err| ConnectionError::Io(io::Error::other(err.to_string())))?;

    let established = handshake::accept(&buffer[..CLIENT_SEED_LEN], &master_secret)
        .map_err(ConnectionError::Handshake)?;
    buffer.drain(..CLIENT_SEED_LEN);

    if trace.on() {
        eprintln!("auth <- client seed ({} bytes)", CLIENT_SEED_LEN);
        eprintln!(
            "auth -> server seed ({} bytes), scrambled from here on",
            established.reply.len()
        );
    }
    stream
        .write_all(&established.reply)
        .await
        .map_err(ConnectionError::Io)?;

    // ---- 4/5. frames, then login ------------------------------------------------------------
    //
    // Note what happens when this connection ends: nothing is written to the database. A session is
    // *not* consumed by logging in - GWPS only ever reads its session table (`Db_GetSession`), and
    // the rig's portal hands out the same fixed pair on every login, so a service that closed the
    // session on disconnect would refuse the client's very next attempt. "The newest session wins"
    // is a rule about live *connections* and about leases (`docs/05-gw2-lessons.md`, rule 1); it is
    // not licence to spend a row.
    let mut session = Session {
        established,
        plaintext: buffer,
        account: None,
        session_id: None,
        character: None,
        server_salt: 0,
        settings_upload: None,
    };

    loop {
        match next_frame(&mut stream, &mut session, trace).await {
            Ok(Some(frame)) => {
                let keep_going =
                    dispatch(&mut stream, &mut session, pool, frame, trace, advertised).await?;
                if !keep_going {
                    break;
                }
            }
            Ok(None) => break, // the client closed the connection
            Err(err) => return Err(err),
        }
    }
    Ok(())
}

/// Handle one decrypted frame. `Ok(false)` means the connection is finished.
async fn dispatch(
    stream: &mut TcpStream,
    session: &mut Session,
    pool: &PgPool,
    frame: Frame,
    trace: Trace,
    advertised: SocketAddr,
) -> Result<bool, ConnectionError> {
    if trace.on() {
        trace_frame("<-", &frame);
    }

    match frame.header() {
        // GWPS `AuthSrv_SendComputerInfo`'s counterpart: the reference stores the reported name and
        // computer name on the connection and sends nothing. (`0x8001` never reaches here - it is
        // consumed structurally before framing; see `messages::decode_computer_info`.)
        tables::AUTH_CMSG_SEND_COMPUTER_INFO => Ok(true),

        // **This is the message the client waits on.** In the reference's dispatch,
        // `AUTH_CMSG_SEND_COMPUTER_HASH` decodes the hash and immediately calls
        // `AuthSrv_SendSessionInfo` - a random server salt in `AUTH_SMSG_SESSION_INFO`. A service that
        // ignores this message leaves the client at "Connecting" with nothing to report, which is
        // exactly what happened here before the reply existed.
        tables::AUTH_CMSG_SEND_COMPUTER_HASH => {
            let salt = random_u32()?;
            session.server_salt = salt;
            if trace.on() {
                eprintln!("auth: computer hash -> session info (salt {salt:#010X})");
            }
            send_frames(
                stream,
                session,
                vec![messages::session_info(salt, 0)],
                trace,
            )
            .await?;
            Ok(true)
        }

        tables::AUTH_CMSG_PORTAL_ACCOUNT_LOGIN => {
            let login = PortalAccountLogin::decode(&frame).map_err(ConnectionError::Message)?;
            if trace.on() {
                eprintln!(
                    "auth: portal login request={} charname={:?} kind={:?}",
                    login.request, login.character_name, login.kind
                );
            }
            let replies = answer_login(pool, session, &login, trace).await?;
            send_frames(stream, session, replies, trace).await?;
            Ok(true)
        }

        // `AuthSrv_HandleAskServerResponse`: answer the request and nothing else.
        tables::AUTH_CMSG_ASK_SERVER_RESPONSE => {
            let request = request_id(&frame);
            send_frames(
                stream,
                session,
                vec![RequestResponse {
                    request,
                    status: STATUS_OK,
                }
                .frame()],
                trace,
            )
            .await?;
            Ok(true)
        }

        // `AuthSrv_HandleClientHeartbeat`: answer with a server heartbeat carrying tick 16, the
        // reference's constant.
        tables::AUTH_CMSG_HEARTBEAT => {
            send_frames(stream, session, vec![messages::heartbeat(16)], trace).await?;
            Ok(true)
        }

        // `AuthSrv_HandleChangePlayCharacter`: find the character by name, record it, answer the
        // request. A name that is not on this account is an error, not a silent success.
        tables::AUTH_CMSG_CHANGE_PLAY_CHARACTER => {
            let request = request_id(&frame);
            let Some(account) = session.account else {
                send_frames(
                    stream,
                    session,
                    vec![RequestResponse {
                        request,
                        status: messages::STATUS_AUTH_ERROR,
                    }
                    .frame()],
                    trace,
                )
                .await?;
                return Ok(true);
            };

            let name = messages::decode_change_character(frame.body())
                .map_err(ConnectionError::Message)?;
            let characters = db_login::characters(pool, account)
                .await
                .map_err(ConnectionError::Database)?;
            let chosen = characters.iter().find(|character| character.name == name);

            let status = match chosen {
                Some(character) => {
                    if let Some(session_id) = session.session_id {
                        let _ = db_login::select_character(pool, session_id, character.id).await;
                    }
                    // The connection now has a character, which is what a travel request needs.
                    session.character = Some(character.id);
                    if trace.on() {
                        eprintln!("auth: character {name:?} selected");
                    }
                    STATUS_OK
                }
                None => {
                    if trace.on() {
                        eprintln!("auth: no character {name:?} on this account");
                    }
                    2 // GM_ERROR_NETWORK_ERROR, which is what the reference answers here
                }
            };
            send_frames(
                stream,
                session,
                vec![RequestResponse { request, status }.frame()],
                trace,
            )
            .await?;
            Ok(true)
        }

        // The client's settings upload, first half: it announces the whole blob. The settings are the
        // client's UI state - window positions, visibility and the other interface options - and the
        // bytes are stored verbatim. `logs/auth.log` shows the pattern: 0x8021 announces 578 bytes,
        // then 0x8020 carries 512 and 66 of them.
        tables::AUTH_CMSG_SETTING_UPDATE_SIZE => {
            let request = request_id(&frame);
            match messages::decode_settings_size(frame.body()) {
                Ok((announced, total))
                    if total > 0 && total as usize <= ACCOUNT_SETTINGS_CAPACITY =>
                {
                    if trace.on() {
                        eprintln!("auth: settings upload {announced} announces {total} bytes");
                    }
                    session.settings_upload = Some(SettingsUpload {
                        request: announced,
                        total,
                        bytes: Vec::with_capacity(total as usize),
                    });
                }
                Ok((announced, total)) => {
                    eprintln!(
                        "auth: settings upload {announced} announces {total} bytes, outside 1..={ACCOUNT_SETTINGS_CAPACITY}; not stored"
                    );
                    session.settings_upload = None;
                }
                Err(err) => {
                    eprintln!("auth: settings size is malformed: {err}");
                    session.settings_upload = None;
                }
            }
            send_frames(
                stream,
                session,
                vec![RequestResponse {
                    request,
                    status: STATUS_OK,
                }
                .frame()],
                trace,
            )
            .await?;
            Ok(true)
        }

        // The client's settings upload, second half: one chunk. When the announced total has
        // arrived, the bytes replace the account's stored blob, and the next login is answered with
        // them (`AUTH_SMSG_ACCOUNT_SETTINGS`).
        tables::AUTH_CMSG_SETTING_UPDATE_CONTENT => {
            let request = request_id(&frame);
            let mut completed = None;
            match messages::decode_settings_content(frame.body()) {
                Ok((chunk_request, chunk)) => {
                    let mut finished = false;
                    match session.settings_upload.as_mut() {
                        Some(upload) if upload.request == chunk_request => {
                            upload.bytes.extend_from_slice(&chunk);
                            finished = upload.bytes.len() >= upload.total as usize;
                        }
                        Some(upload) => eprintln!(
                            "auth: settings chunk for request {chunk_request} while request {} is open; ignored",
                            upload.request
                        ),
                        None => eprintln!(
                            "auth: settings chunk for request {chunk_request} without a size announcement; ignored"
                        ),
                    }
                    if finished {
                        completed = session.settings_upload.take().map(|upload| upload.bytes);
                    }
                }
                Err(err) => eprintln!("auth: settings content is malformed: {err}"),
            }
            if let Some(bytes) = completed {
                match session.account {
                    Some(account) => match db_account::record_settings(pool, account, &bytes).await {
                        Ok(true) => {
                            if trace.on() {
                                eprintln!(
                                    "auth: stored {}-byte account settings for {account}",
                                    bytes.len()
                                );
                            }
                        }
                        Ok(false) => eprintln!(
                            "auth: settings upload for {account} matched no account row"
                        ),
                        Err(err) => eprintln!(
                            "auth: could not store the account settings for {account}: {err}"
                        ),
                    },
                    None => eprintln!(
                        "auth: settings upload before the account resolved; not stored"
                    ),
                }
            }
            send_frames(
                stream,
                session,
                vec![RequestResponse {
                    request,
                    status: STATUS_OK,
                }
                .frame()],
                trace,
            )
            .await?;
            Ok(true)
        }

        // The other settings upload, the character-select blob (`0x8009`), which the world path
        // already keeps: acknowledged and nothing more. An unacknowledged settings upload is a hang,
        // and GWPS lived through seven of those (`docs/03-protocol.md`, section 6).
        tables::AUTH_CMSG_UPDATE_CHARACTER_SETTINGS => {
            let request = request_id(&frame);
            let reply = RequestResponse {
                request,
                status: STATUS_OK,
            };
            send_frames(stream, session, vec![reply.frame()], trace).await?;
            Ok(true)
        }

        // `AuthSrv_HandleAddAccessKey`: the reference decodes the key to a string, **logs it and stores
        // nothing**, then answers the request with status 0. Parity means exactly that - an access key
        // that vanished after being logged. Giving it storage would invent behaviour GWPS does not have.
        tables::AUTH_CMSG_ADD_ACCESS_KEY => {
            let request = request_id(&frame);
            if trace.on() {
                eprintln!(
                    "auth <- access key, {} key unit(s) (the reference logs it and keeps nothing)",
                    access_key_len(frame.body())
                );
            }
            send_frames(
                stream,
                session,
                vec![RequestResponse {
                    request,
                    status: STATUS_OK,
                }
                .frame()],
                trace,
            )
            .await?;
            Ok(true)
        }

        // `AuthSrv_HandleDeleteCharacter`: the name must be one of *this account's* characters, then the
        // row goes and the request is answered with status 0. The reference answers 23
        // (`GM_ERROR_INVALID_CHARNAME`) for a name it does not know, and 2 if the delete itself fails,
        // and it does **not** re-send the character list.
        tables::AUTH_CMSG_DELETE_CHARACTER => {
            let request = request_id(&frame);
            let status = match session.account {
                None => messages::STATUS_NETWORK_ERROR,
                Some(account) => {
                    let name = messages::decode_change_character(frame.body())
                        .map_err(ConnectionError::Message)?;
                    let characters = db_login::characters(pool, account)
                        .await
                        .map_err(ConnectionError::Database)?;
                    match characters.iter().find(|character| character.name == name) {
                        None => {
                            if trace.on() {
                                eprintln!("auth: no character {name:?} to delete on this account");
                            }
                            messages::STATUS_INVALID_CHARNAME
                        }
                        Some(character) => {
                            let deleted = character.id;
                            match db_login::delete_character(pool, account, deleted).await {
                                Ok(true) => {
                                    if trace.on() {
                                        eprintln!("auth: character {name:?} deleted");
                                    }
                                    // This connection must stop naming it: the next travel - a
                                    // creation instance or a map - carries `session.character` into
                                    // the transfer row, and the world refuses a character that is
                                    // gone ("the character named by the transfer is gone", measured
                                    // 2026-10-28 right after a delete). The account's first remaining
                                    // character takes its place, which is what the login would choose.
                                    if session.character == Some(deleted) {
                                        let remaining = db_login::characters(pool, account)
                                            .await
                                            .map_err(ConnectionError::Database)?;
                                        session.character =
                                            remaining.first().map(|character| character.id);
                                        eprintln!(
                                            "auth: the deleted character was this session's; it \
                                             now names {:?}",
                                            session.character
                                        );
                                    }
                                    STATUS_OK
                                }
                                Ok(false) => messages::STATUS_NETWORK_ERROR,
                                Err(err) => {
                                    eprintln!("auth: could not delete {name:?}: {err}");
                                    messages::STATUS_NETWORK_ERROR
                                }
                            }
                        }
                    }
                }
            };
            send_frames(
                stream,
                session,
                vec![RequestResponse { request, status }.frame()],
                trace,
            )
            .await?;
            Ok(true)
        }

        // The reference keeps the client's player status on the connection and sends nothing.
        tables::AUTH_CMSG_SET_PLAYER_STATUS => Ok(true),

        // The client accepted the EULA and names the version it accepted (`1a` = 26 against build
        // 39112). Recording it is the whole point: the account info of the next login carries it back
        // and the client stops asking (`crates/asura-db/migrations/0012_account_eula.sql`).
        //
        // Nothing is sent in reply - the reference sends nothing here and the client waits for
        // nothing. A write that fails is logged and swallowed: the login already succeeded, and a
        // refusal now would only make the client ask again.
        tables::AUTH_CMSG_ACCEPT_EULA => {
            let version = messages::decode_eula(frame.body()).map_err(ConnectionError::Message)?;
            if let Some(account) = session.account {
                match db_account::record_eula(pool, account, i16::from(version)).await {
                    // Printed, not traced: "this account accepted the EULA" is an event, and the
                    // trace is the thing that buries events three lines deep in a 500 ms tick.
                    Ok(true) => eprintln!("auth: {account} accepted EULA version {version}"),
                    Ok(false) => eprintln!("auth: no account row {account} to record the EULA on"),
                    Err(err) => eprintln!("auth: could not record the EULA of {account}: {err}"),
                }
            }
            Ok(true)
        }

        // A travel request is answered with a world that is actually reachable *now*
        // (`travel::answer`): the info message with an address, then the success response. The one
        // rule it exists to keep is that the client is never sent to a port nothing answers on.
        tables::AUTH_CMSG_REQUEST_GAME_INSTANCE => {
            let request = messages::GameInstanceRequest::decode(frame.body())
                .map_err(ConnectionError::Message)?;
            let replies = travel::answer(
                pool,
                session.session_id,
                session.account,
                session.character,
                request,
                advertised,
            )
            .await?;
            send_frames(stream, session, replies, trace).await?;
            Ok(true)
        }

        // The client is going away; the reference drops the connection.
        tables::AUTH_CMSG_DISCONNECT => Ok(false),

        // Anything else is logged and skipped rather than closing the connection: the reference
        // warns and carries on, and dropping a connection over a message we simply do not use yet
        // would make the client's symptom worse than the cause.
        other => {
            eprintln!(
                "auth: unhandled header {other:#06X} ({} bytes)",
                frame.len()
            );
            Ok(true)
        }
    }
}

/// The number of UTF-16 units in an `AUTH_CMSG_ADD_ACCESS_KEY` body, after its request id.
///
/// The reference decodes the same field into a string and logs it (`AuthSrv_HandleAddAccessKey`); this
/// service only counts it, because it stores nothing and a key in a log is a secret in a log.
fn access_key_len(body: &[u8]) -> usize {
    if body.len() < 6 {
        return 0;
    }
    u16::from_le_bytes([body[4], body[5]]) as usize
}

/// The request id a message carries in its first four body bytes.
///
/// Every `AUTH_CMSG` the dispatch answers this way has `req_id` first (`change_character`,
/// `ask_server_response`, the settings updates, `request_game_instance`), so one helper reads it
/// rather than five copies of the same four-byte dance.
fn request_id(frame: &Frame) -> u32 {
    let body = &frame.body();
    if body.len() < 4 {
        return 0;
    }
    u32::from_le_bytes([body[0], body[1], body[2], body[3]])
}

/// The committed settings blob a login is answered with when the account has never uploaded one
/// (`config/Server/account_settings.json`).
fn committed_account_settings() -> Result<Vec<u8>, String> {
    asura_config::account_settings().and_then(|settings| settings.bytes())
}

/// A random `u32` for the session salt, from the operating system.
pub(crate) fn random_u32() -> Result<u32, ConnectionError> {
    let mut bytes = [0u8; 4];
    getrandom::getrandom(&mut bytes)
        .map_err(|err| ConnectionError::Io(io::Error::other(err.to_string())))?;
    Ok(u32::from_le_bytes(bytes))
}

/// The login decision, expressed as the frames to send back.
async fn answer_login(
    pool: &PgPool,
    session: &mut Session,
    login: &PortalAccountLogin,
    trace: Trace,
) -> Result<Vec<Frame>, ConnectionError> {
    // The UUIDs travel in the little-endian RFC 4122 variant; reading them the ordinary way would
    // make every id wrong (`client::messages`, module documentation).
    let portal_user = Uuid::from_bytes_le(login.user_id);
    let session_id = Uuid::from_bytes_le(login.session_id);

    let resolved = db_login::find_session(pool, portal_user, session_id)
        .await
        .map_err(ConnectionError::Database)?;

    let Some(resolved) = resolved else {
        if trace.on() {
            eprintln!("auth: no session for {portal_user} / {session_id}; refusing");
        }
        // The client shows this as an auth error. The connection stays open: the reference answers
        // the request and waits, and so does this.
        session.account = None;
        return Ok(vec![RequestResponse {
            request: login.request,
            status: messages::STATUS_AUTH_ERROR,
        }
        .frame()]);
    };

    // The session table is read, never spent: GWPS's login path calls `Db_GetSession` and writes
    // nothing, and the rig's portal presents the same fixed pair on every login. Closing rows here
    // would refuse the client's next attempt. The "newest wins" rule applies to live connections and
    // to leases, not to this row (`docs/03-protocol.md` section 4).
    let characters = db_login::characters(pool, resolved.account)
        .await
        .map_err(ConnectionError::Database)?;

    // The EULA version this account accepted, which the account info carries back so the client does
    // not ask again. A read that fails is not worth losing a login over: 0 asks the client once, and
    // its answer is recorded, so a transient error costs one extra page rather than a session.
    //
    // The same row holds the client's uploaded UI settings (`t_account.account_settings`), which the
    // login answer carries instead of the committed default when it is present.
    let (eula_accepted, stored_settings) = match db_account::load(pool, resolved.account).await {
        Ok(Some(account)) => (
            account.eula_accepted.max(0) as u8,
            account.account_settings,
        ),
        Ok(None) => {
            eprintln!(
                "auth: no account row {} for a session that resolved",
                resolved.account
            );
            (0, None)
        }
        Err(err) => {
            eprintln!(
                "auth: could not read the account of {}: {err}",
                resolved.account
            );
            (0, None)
        }
    };

    session.account = Some(resolved.account);
    // The pair resolved, so the session row is real and this connection may act on it: a travel
    // request needs it for the lease, and a character selection is recorded on it. Leaving this unset
    // made every travel refuse with "the connection has no session".
    session.session_id = Some(session_id);
    if resolved.character.is_some() && trace.on() {
        eprintln!(
            "auth: the session records character {:?} as last selected",
            resolved.character
        );
    }

    // The order GWPS sends, which the client expects (`AuthSrv_HandlePortalAccountLogin`):
    // the characters, the stored settings, the friend-list terminator, the account info, and finally
    // the answer to the request.
    //
    // A session can name a character that no longer exists: deleting a character leaves its id in
    // `session.char_id`, and every travel then names a ghost - measured 2026-10-28, after five
    // characters were deleted the travel went out for `59935da2-...` and the world answered "the
    // character named by the transfer is gone". Only an id still on the account is kept.
    let selected = resolved
        .character
        .filter(|id| characters.iter().any(|character| character.id == *id))
        .or_else(|| characters.first().map(|character| character.id));
    // The reference falls back the same way when a session records no character: the first character
    // on the account is what a travel would put in a map, and refusing to pick one leaves the client
    // at character select with a travel it cannot complete.
    if let Some(selected) = selected {
        if resolved.character != Some(selected) {
            match db_login::select_character(pool, session_id, selected).await {
                Ok(_) => eprintln!(
                    "auth: the session named a character that is gone; it now names {selected}"
                ),
                Err(err) => eprintln!("auth: could not record the session's character: {err}"),
            }
        }
    }
    session.character = selected;
    let mut replies = Vec::with_capacity(characters.len() + 4);
    for character in &characters {
        // The settings block, built from this character's row (`CharacterSettings_FromDbCharacter`,
        // `GmChar.c`) rather than the constant that used to stand in for every character: the list is
        // where the client gets the appearance, level, campaign and armour it draws at character select.
        let settings = match db_game::settings(pool, character.id).await {
            Ok(Some(settings)) => messages::character_settings(&messages::CharacterSettings {
                // The blob's `last_outpost` is the reference's `DbCharacter.last_outpost`: **the map the
                // character is in**, which this schema keeps as `characters.map_id`. It is not our own
                // `characters.last_outpost` column (migration `0024`), which remembers the outpost the
                // login rule places a character in. The two differ the moment a character has never
                // been placed in an outpost, and then the client draws it as map 0 with no data.
                last_outpost: u16::try_from(settings.map_id).unwrap_or(0),
                appearance: settings.appearance,
                campaign: settings.campaign,
                level: settings.level,
                is_pvp: settings.is_pvp,
                secondary_profession: settings.secondary_profession,
                helm_status: settings.helm_status,
                pieces: settings
                    .pieces
                    .iter()
                    .map(|piece| messages::ArmourPiece {
                        file_id: piece.file_id,
                        colors: piece.colors,
                    })
                    .collect(),
            }),
            Ok(None) => Vec::new(),
            Err(err) => {
                // A settings read that fails is not worth losing a login over: the client gets the row it
                // already knows (nothing) and the list still arrives.
                eprintln!(
                    "auth: could not read the settings of {}: {err}",
                    character.id
                );
                Vec::new()
            }
        };
        replies.push(
            messages::character_info(
                login.request,
                character.id.to_bytes_le(),
                &character.name,
                &settings,
            )
            .map_err(ConnectionError::Message)?,
        );
    }
    // The account's own uploaded settings blob, when there is one, is what the login is answered
    // with: the client's UI state - window positions, visibility and the interface options - belongs
    // to the account, and retail saves it that way. The committed blob is the first-login default,
    // and the fallback when the stored one cannot be carried in one answer.
    let account_settings = match stored_settings {
        Some(bytes) if !bytes.is_empty() && bytes.len() <= ACCOUNT_SETTINGS_CAPACITY => {
            if trace.on() {
                eprintln!(
                    "auth: account settings from the account, {} bytes",
                    bytes.len()
                );
            }
            Ok(bytes)
        }
        Some(bytes) => {
            eprintln!(
                "auth: the account settings blob is {} bytes, outside 1..={ACCOUNT_SETTINGS_CAPACITY}; using the committed blob",
                bytes.len()
            );
            committed_account_settings()
        }
        None => committed_account_settings(),
    };
    let account_settings = match account_settings {
        Ok(bytes) => bytes,
        Err(err) => {
            eprintln!("auth: the account settings blob is not usable: {err}");
            return Ok(replies);
        }
    };
    replies.push(messages::account_settings(login.request, &account_settings));
    replies.push(messages::friend_stream_end(login.request));
    replies.push(messages::account_info(
        login.request,
        resolved.account.to_bytes_le(),
        selected.map(|character| character.to_bytes_le()),
        eula_accepted,
    ));
    replies.push(
        RequestResponse {
            request: login.request,
            status: STATUS_OK,
        }
        .frame(),
    );

    // Not behind the trace: "who just logged in" is the single line a person watching the console needs,
    // and a packet trace is the thing that buries it.
    eprintln!(
        "auth: login ok for {} with {} character(s)",
        resolved.account,
        characters.len()
    );
    Ok(replies)
}

/// Scramble and write frames.
async fn send_frames(
    stream: &mut TcpStream,
    session: &mut Session,
    frames: Vec<Frame>,
    trace: Trace,
) -> Result<(), ConnectionError> {
    for frame in frames {
        if trace.on() {
            trace_frame("->", &frame);
        }
        // No table-size validation: messages are variable-length (u16 counts, no padding), so the
        // table`s sizes are maxima. `encode` writes what the frame holds; the client frames it by the
        // same field-walking rule the service uses on the way in.
        let mut bytes = frame.encode();
        session.established.outgoing.apply(&mut bytes);
        stream
            .write_all(&bytes)
            .await
            .map_err(ConnectionError::Io)?;
    }
    Ok(())
}

/// The next frame, reading from the socket as needed. `Ok(None)` is a clean disconnect.
async fn next_frame(
    stream: &mut TcpStream,
    session: &mut Session,
    trace: Trace,
) -> Result<Option<Frame>, ConnectionError> {
    loop {
        // Client messages are framed by *walking their field layout*, not by the table's maximum size:
        // a counted field carries a u16 count and occupies only that much
        // (`Frame::decode_client_message`). Framing them by the table is what made a real client hang -
        // its login message is 54 bytes, and the table says 150.
        match Frame::decode_client_message(&session.plaintext) {
            Ok((frame, used)) => {
                session.plaintext.drain(..used);
                return Ok(Some(frame));
            }
            Err(FrameError::Incomplete { .. }) => {
                if trace.on() {
                    trace_waiting(&session.plaintext);
                }
            }
            Err(err) => return Err(ConnectionError::Framing(err)),
        }

        let mut chunk = [0u8; READ_CHUNK];
        let read = match tokio::time::timeout(idle_timeout(), stream.read(&mut chunk)).await {
            Err(_) => return Err(ConnectionError::Idle),
            Ok(Ok(0)) => return Ok(None),
            Ok(Ok(read)) => read,
            Ok(Err(err)) => return Err(ConnectionError::Io(err)),
        };

        let mut plaintext = chunk[..read].to_vec();
        session.established.incoming.apply(&mut plaintext);
        session.plaintext.extend_from_slice(&plaintext);
        if trace.on() {
            // The whole chunk, not a preview. When the client and the table disagree about a message,
            // these are the bytes that settle it, and there is no second chance to capture them.
            eprintln!("auth <- {read} byte(s) on the wire, decrypted to:");
            for line in plaintext.chunks(16) {
                let hex: String = line
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                let text: String = line
                    .iter()
                    .map(|byte| {
                        if byte.is_ascii_graphic() {
                            *byte as char
                        } else {
                            '.'
                        }
                    })
                    .collect();
                eprintln!("         {hex:<48} {text}");
            }
        }
    }
}

/// Say what the undecoded plaintext claims to be.
///
/// This is the line that answers "the client is waiting and we are waiting": it prints the two bytes
/// being read as a header, the size they declare, and whether that header is in the table. A wrong key
/// stream shows up here as a header nobody sent, which is much cheaper than guessing from the client's
/// `007`.
fn trace_waiting(plaintext: &[u8]) {
    if plaintext.len() < 2 {
        eprintln!(
            "auth .. waiting: {} byte(s) buffered, not even a header",
            plaintext.len()
        );
        return;
    }
    let header = u16::from_le_bytes([plaintext[0], plaintext[1]]);
    let name = tables::entry(header)
        .and_then(|entry| entry.name)
        .unwrap_or("NOT IN THE TABLE");
    let expected = tables::expected_size(header);
    eprintln!(
        "auth .. waiting: {} byte(s) buffered, header {:#06X} ({name}), table size {:?}",
        plaintext.len(),
        header,
        expected
    );
}

/// Read into `buffer` until it holds at least `wanted` bytes.
///
/// The buffer is the caller's, and it is *not* consumed here: the first four bytes of a connection
/// are the start of the version message, so discarding them after classifying the connection would
/// lose the message that was being classified.
async fn read_until(
    stream: &mut TcpStream,
    buffer: &mut Vec<u8>,
    wanted: usize,
) -> Result<(), ConnectionError> {
    while buffer.len() < wanted {
        let mut chunk = [0u8; READ_CHUNK];
        let read = match tokio::time::timeout(idle_timeout(), stream.read(&mut chunk)).await {
            Err(_) => return Err(ConnectionError::Idle),
            Ok(Ok(0)) => {
                return Err(ConnectionError::Io(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "the client closed the connection mid-handshake",
                )))
            }
            Ok(Ok(read)) => read,
            Ok(Err(err)) => return Err(ConnectionError::Io(err)),
        };
        buffer.extend_from_slice(&chunk[..read]);
    }
    Ok(())
}

/// One trace line: direction, header, name, size and the first bytes (`docs/03-protocol.md` #6).
fn trace_frame(direction: &str, frame: &Frame) {
    let name = tables::entry(frame.header())
        .and_then(|entry| entry.name)
        .unwrap_or("unnamed");
    let preview: String = frame
        .body()
        .iter()
        .take(16)
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(" ");
    eprintln!(
        "auth {direction} {:#06X} {name} {} bytes: {preview}",
        frame.header(),
        frame.len()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_trace_switch_accepts_what_the_reference_accepts() {
        assert!(Trace::is_on_value("1"));
        assert!(Trace::is_on_value("true"));
        assert!(Trace::is_on_value("TRUE"));
        assert!(Trace::is_on_value(" yes "));

        assert!(!Trace::is_on_value(""));
        assert!(!Trace::is_on_value("0"));
        assert!(!Trace::is_on_value("off"));
        assert!(!Trace::is_on_value("no"));
    }
}
