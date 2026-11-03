//! `asura-webgate`: the portal web gate, and the one thing that must be answered before a login.
//!
//! The client launched with `-portal 127.0.0.1` does not ask an account database who it is. It calls
//! four HTTP endpoints on the gate and takes its identity from the answers
//! (`docs/03-protocol.md` #4, `docs/06-reference-session.md`):
//!
//! | request | answer |
//! |---|---|
//! | `GET  /Spawned/WebGate/session/create.xml` | `<Reply><Session>UUID</Session></Reply>` |
//! | `POST /Spawned/WebGate/users/login.xml` | the `UserId`/`LoginName`/`Provider` block |
//! | `POST /Spawned/WebGate/my_account/game_accounts.xml` | a one-row array for the game code |
//! | `POST /Spawned/WebGate/my_account/token.xml` | `<Reply><Token>UUID</Token></Reply>` |
//!
//! Plus one route this project adds for its own supervision, not the reference's: `GET /health`
//! answers `200 ok` so a supervisor can ask "is the gate up?" without a session.
//!
//! This is a faithful port of the reference's `tools/webgate.py`, with one deliberate change: the
//! reference answers from a hard-coded table of three emails, and this answers from the rows
//! `tools/seed-dev.ps1` writes (the same pairs), through `asura_db::portal::identity_for_email`. The
//! wire behaviour is the reference's: a session cookie is required for every endpoint after the
//! first, an unknown email answers `400 who are you?`, a non-`Portal` provider answers
//! `400 Unsupported provider`, and a missing session answers `401`.
//!
//! ## The session, and why it is in memory
//!
//! `session/create.xml` mints a UUID and `users/login.xml` attaches the identity to it; that pair of
//! steps is the reference's `sessions` dict, and it is kept here the same way - one process, a map of
//! live sessions. The *identity* is not invented here: it is the exact `(portal_user, session id)`
//! pair the auth service will resolve out of the database (`asura_db::portal`), so a gate that has
//! forgotten a session costs a retry, never a login as the wrong account.

use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use uuid::Uuid;

/// The pair the client is handed, and the pair the auth service resolves (`account.session`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortalUser {
    /// The portal user id: `session.portal_user`.
    pub user_id: Uuid,
    /// The session id: `session.id`.
    pub token: Uuid,
}

/// Where the gate gets identities. The database in production; a fixed table in the tests.
#[derive(Clone)]
enum Identities {
    Database(asura_db::PgPool),
    /// `email -> (portal_user, session id)`, for tests that must not need PostgreSQL.
    Fixed(Arc<HashMap<String, (Uuid, Uuid)>>),
}

impl Identities {
    async fn resolve(&self, email: &str) -> Result<Option<PortalUser>, String> {
        match self {
            Identities::Database(pool) => asura_db::portal::identity_for_email(pool, email)
                .await
                .map(|found| found.map(|(user_id, token)| PortalUser { user_id, token }))
                .map_err(|err| err.to_string()),
            Identities::Fixed(table) => {
                Ok(table.get(email.trim()).map(|(user_id, token)| PortalUser {
                    user_id: *user_id,
                    token: *token,
                }))
            }
        }
    }
}

/// One gate: the identities it answers with, and the sessions it has minted.
///
/// Cheap to clone; every connection gets one.
#[derive(Clone)]
pub struct Gate {
    identities: Identities,
    sessions: Arc<Mutex<HashMap<String, Option<PortalUser>>>>,
}

impl Gate {
    /// A gate backed by the database.
    pub fn database(pool: asura_db::PgPool) -> Gate {
        Gate {
            identities: Identities::Database(pool),
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// A gate backed by a fixed table, for tests: `email -> (portal_user, session id)`.
    pub fn fixed(table: HashMap<String, (Uuid, Uuid)>) -> Gate {
        Gate {
            identities: Identities::Fixed(Arc::new(table)),
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Whether a caller may use the session id: it must have been minted by this gate.
    async fn session_exists(&self, id: &str) -> bool {
        self.sessions.lock().await.contains_key(id)
    }

    /// The identity attached to a session, if its login ran.
    async fn session_user(&self, id: &str) -> Option<PortalUser> {
        self.sessions.lock().await.get(id).copied().flatten()
    }

    async fn attach(&self, id: &str, user: PortalUser) {
        if let Some(slot) = self.sessions.lock().await.get_mut(id) {
            *slot = Some(user);
        }
    }

    /// One HTTP request, answered. The route table is the reference's `do_GET`/`do_POST`.
    async fn route(
        &self,
        method: &str,
        path: &str,
        authorization: Option<&str>,
        body: &[u8],
    ) -> Reply {
        let session_id = authorization
            .and_then(|value| value.strip_prefix("Arena "))
            .map(str::trim);

        match (method, path) {
            ("GET", "/Spawned/WebGate/session/create.xml") => {
                let id = Uuid::new_v4();
                let text = upper(id);
                self.sessions.lock().await.insert(text.clone(), None);
                Reply::xml(xml_session_created(&text))
            }

            ("POST", "/Spawned/WebGate/users/login.xml") => {
                let Some(id) = session_id.filter(|id| !id.is_empty()) else {
                    return Reply::unauthorized();
                };
                // Cheap check first so a bad cookie never reaches the database.
                if !{
                    let sessions = self.sessions.lock().await;
                    sessions.contains_key(id)
                } {
                    return Reply::unauthorized();
                }
                let text = String::from_utf8_lossy(body).into_owned();
                let provider = field(&text, "Provider").unwrap_or_default();
                if provider != "Portal" {
                    return Reply::bad_request(b"Unsupported provider");
                }
                let Some(email) = field(&text, "LoginName").filter(|email| !email.is_empty())
                else {
                    return Reply::bad_request(b"who are you?");
                };
                let Some(user) = self.identities.resolve(email).await.ok().flatten() else {
                    return Reply::bad_request(b"who are you?");
                };
                self.attach(id, user).await;
                let user_id = upper(user.user_id);
                Reply::xml(xml_login(&user_id, email))
            }

            ("POST", "/Spawned/WebGate/my_account/game_accounts.xml") => {
                let Some(id) = session_id.filter(|id| !id.is_empty()) else {
                    return Reply::unauthorized();
                };
                if !self.session_exists(id).await {
                    return Reply::unauthorized();
                }
                let text = String::from_utf8_lossy(body).into_owned();
                let game_code = field(&text, "GameCode").unwrap_or_default();
                Reply::xml(xml_game_accounts(game_code))
            }

            ("POST", "/Spawned/WebGate/my_account/token.xml") => {
                let Some(id) = session_id.filter(|id| !id.is_empty()) else {
                    return Reply::unauthorized();
                };
                let Some(user) = self.session_user(id).await else {
                    // The reference crashes on this (`session.token_id` is unset); a clean 401 is the
                    // same refusal a client can act on: log in first.
                    return Reply::unauthorized();
                };
                Reply::xml(xml_token(&upper(user.token)))
            }

            // This project's own addition: the supervisor's health check. It is not one of the
            // reference's routes, and it answers without a session - it says "the gate is up",
            // nothing else.
            ("GET", "/health") => Reply::ok(),

            _ => Reply::not_found(),
        }
    }
}

/// What one answer is: status line, content type (the reference sets `application/xml` on every
/// answer), and the body.
#[derive(Debug, PartialEq, Eq)]
struct Reply {
    status: &'static str,
    content_type: Option<&'static str>,
    body: Vec<u8>,
}

impl Reply {
    fn xml(body: String) -> Reply {
        Reply {
            status: "200 OK",
            content_type: Some("application/xml"),
            body: body.into_bytes(),
        }
    }

    /// The supervisor's health answer: the gate is up. Not one of the reference's routes.
    fn ok() -> Reply {
        Reply {
            status: "200 OK",
            content_type: Some("text/plain"),
            body: b"ok".to_vec(),
        }
    }

    fn bad_request(body: &[u8]) -> Reply {
        Reply {
            status: "400 Bad Request",
            content_type: None,
            body: body.to_vec(),
        }
    }

    fn unauthorized() -> Reply {
        Reply {
            status: "401 Unauthorized",
            content_type: None,
            body: Vec::new(),
        }
    }

    fn not_found() -> Reply {
        Reply {
            status: "404 Not Found",
            content_type: None,
            body: Vec::new(),
        }
    }
}

/// The biggest request head this gate will read. The client's calls are tiny; a larger head is a
/// client that is not this client.
const MAX_HEAD: usize = 64 * 1024;

/// Accept connections forever, answering each on its own task.
pub async fn serve(listener: TcpListener, gate: Gate) -> io::Result<()> {
    loop {
        let (stream, _peer) = listener.accept().await?;
        let gate = gate.clone();
        tokio::spawn(async move {
            // A connection that ends in an I/O error is a client that went away; the gate is shared
            // state, and one dead socket is not a reason to stop serving.
            let _ = handle(stream, gate).await;
        });
    }
}

async fn handle(mut stream: TcpStream, gate: Gate) -> io::Result<()> {
    // One request per connection, as the reference's `SimpleHTTPRequestHandler` (HTTP/1.0, close
    // after every answer).
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            return Ok(());
        }
        buf.extend_from_slice(&chunk[..read]);
        if let Some(position) = buf.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
        if buf.len() > MAX_HEAD {
            return Ok(());
        }
    };

    let head = String::from_utf8_lossy(&buf[..head_end - 4]).into_owned();
    let mut lines = head.split("\r\n");
    let Some(request_line) = lines.next() else {
        return Ok(());
    };
    let mut parts = request_line.split_whitespace();
    let (Some(method), Some(path)) = (parts.next(), parts.next()) else {
        return Ok(());
    };
    let mut headers: HashMap<String, String> = HashMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }

    // The body: what arrived with the head, then the rest of Content-Length.
    let wanted: usize = headers
        .get("content-length")
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let mut body = buf.get(head_end..).map(<[u8]>::to_vec).unwrap_or_default();
    while body.len() < wanted {
        let read = stream.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }
    body.truncate(wanted);

    let reply = gate
        .route(
            method,
            path,
            headers.get("authorization").map(String::as_str),
            &body,
        )
        .await;

    let mut out = format!(
        "HTTP/1.0 {}\r\nContent-Length: {}\r\nConnection: close\r\n",
        reply.status,
        reply.body.len()
    );
    if let Some(content_type) = reply.content_type {
        out.push_str(&format!("Content-Type: {content_type}\r\n"));
    }
    out.push_str("\r\n");
    out.push_str(&String::from_utf8_lossy(&reply.body));
    stream.write_all(out.as_bytes()).await?;
    stream.flush().await
}

/// A UUID in the form the reference prints: upper case and hyphenated.
fn upper(id: Uuid) -> String {
    id.to_string().to_uppercase()
}

/// The text between `<tag>` and `</tag>`, the first occurrence, if any.
///
/// The requests are one level deep with no repeated tags in the fields this gate reads, so this is
/// exactly what the reference's `ElementTree.find(...).text` answers.
fn field<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(&xml[start..end])
}

fn xml_session_created(session: &str) -> String {
    format!("<Reply>\n<Session>{session}</Session>\n</Reply>\n")
}

fn xml_login(user_id: &str, email: &str) -> String {
    format!(
        "<Reply>\n\
         <UserId>{user_id}</UserId>\n\
         <UserCenter>1</UserCenter>\n\
         <UserName>:John.10</UserName>\n\
         <Parts/>\n\
         <ResumeToken>AAAAAAAA-BBBB-CCCC-DDDD-FFFFFFFFFFFF</ResumeToken>\n\
         <LoginName>{email}</LoginName>\n\
         <Provider>Portal</Provider>\n\
         <EmailVerified>1</EmailVerified>\n\
         </Reply>\n"
    )
}

fn xml_game_accounts(game_code: &str) -> String {
    format!(
        "<Reply type=\"array\">\n\
         <Row>\n\
         <GameCode>{game_code}</GameCode>\n\
         <Alias>gw1</Alias>\n\
         <Created>2005-01-01T12:00:00Z</Created>\n\
         </Row>\n\
         </Reply>\n"
    )
}

fn xml_token(token: &str) -> String {
    format!("<Reply>\n<Token>{token}</Token>\n</Reply>\n")
}

/// Parse a socket address, with the error kept as text for the CLI.
pub fn parse_bind(value: &str) -> Result<SocketAddr, String> {
    value
        .parse()
        .map_err(|err| format!("{value:?} is not an address: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOGIN_BODY: &str =
        "<Users><Provider>Portal</Provider><LoginName>user1@example.com</LoginName>\
                              <Password>cGFzcw==</Password><GameCode>GW1</GameCode></Users>";

    fn gate() -> Gate {
        let user_id = Uuid::parse_str("fa520ee2-4419-4eb4-ae49-6e9abe6ef24f").unwrap();
        let token = Uuid::parse_str("d8b9bf5d-90b1-4cbd-9b76-88da7be763b6").unwrap();
        let mut table = HashMap::new();
        table.insert("user1@example.com".to_string(), (user_id, token));
        Gate::fixed(table)
    }

    async fn create(gate: &Gate) -> String {
        let reply = gate
            .route("GET", "/Spawned/WebGate/session/create.xml", None, b"")
            .await;
        assert_eq!(reply.status, "200 OK");
        let body = String::from_utf8(reply.body).unwrap();
        field(&body, "Session").expect("a session id").to_string()
    }

    #[tokio::test]
    async fn health_is_answered_without_a_session() {
        let gate = gate();
        let reply = gate.route("GET", "/health", None, b"").await;
        assert_eq!(reply.status, "200 OK");
        assert_eq!(reply.body, b"ok".to_vec());
    }

    #[tokio::test]
    async fn the_four_endpoints_answer_what_the_reference_answers() {
        let gate = gate();
        let session = create(&gate).await;
        assert_eq!(session.len(), 36, "upper-case hyphenated UUID: {session}");

        let reply = gate
            .route(
                "POST",
                "/Spawned/WebGate/users/login.xml",
                Some(&format!("Arena {session}")),
                LOGIN_BODY.as_bytes(),
            )
            .await;
        assert_eq!(reply.status, "200 OK");
        assert_eq!(reply.content_type, Some("application/xml"));
        let body = String::from_utf8(reply.body).unwrap();
        assert_eq!(
            body,
            "<Reply>\n\
             <UserId>FA520EE2-4419-4EB4-AE49-6E9ABE6EF24F</UserId>\n\
             <UserCenter>1</UserCenter>\n\
             <UserName>:John.10</UserName>\n\
             <Parts/>\n\
             <ResumeToken>AAAAAAAA-BBBB-CCCC-DDDD-FFFFFFFFFFFF</ResumeToken>\n\
             <LoginName>user1@example.com</LoginName>\n\
             <Provider>Portal</Provider>\n\
             <EmailVerified>1</EmailVerified>\n\
             </Reply>\n"
        );

        let reply = gate
            .route(
                "POST",
                "/Spawned/WebGate/my_account/token.xml",
                Some(&format!("Arena {session}")),
                b"<Request><GameCode>GW1</GameCode><AccountAlias>gw1</AccountAlias></Request>",
            )
            .await;
        assert_eq!(reply.status, "200 OK");
        assert_eq!(
            String::from_utf8(reply.body).unwrap(),
            "<Reply>\n<Token>D8B9BF5D-90B1-4CBD-9B76-88DA7BE763B6</Token>\n</Reply>\n"
        );

        let reply = gate
            .route(
                "POST",
                "/Spawned/WebGate/my_account/game_accounts.xml",
                Some(&format!("Arena {session}")),
                b"<Request><GameCode>GW1</GameCode></Request>",
            )
            .await;
        assert_eq!(reply.status, "200 OK");
        assert_eq!(
            String::from_utf8(reply.body).unwrap(),
            "<Reply type=\"array\">\n\
             <Row>\n\
             <GameCode>GW1</GameCode>\n\
             <Alias>gw1</Alias>\n\
             <Created>2005-01-01T12:00:00Z</Created>\n\
             </Row>\n\
             </Reply>\n"
        );
    }

    #[tokio::test]
    async fn an_unknown_email_is_400_who_are_you() {
        let gate = gate();
        let session = create(&gate).await;
        let reply = gate
            .route(
                "POST",
                "/Spawned/WebGate/users/login.xml",
                Some(&format!("Arena {session}")),
                b"<Users><Provider>Portal</Provider><LoginName>nobody@example.com</LoginName></Users>",
            )
            .await;
        assert_eq!(reply.status, "400 Bad Request");
        assert_eq!(reply.body, b"who are you?");
    }

    #[tokio::test]
    async fn a_non_portal_provider_is_refused() {
        let gate = gate();
        let session = create(&gate).await;
        let reply = gate
            .route(
                "POST",
                "/Spawned/WebGate/users/login.xml",
                Some(&format!("Arena {session}")),
                b"<Users><Provider>Steam</Provider><LoginName>user1@example.com</LoginName></Users>",
            )
            .await;
        assert_eq!(reply.status, "400 Bad Request");
        assert_eq!(reply.body, b"Unsupported provider");
    }

    #[tokio::test]
    async fn a_session_is_required_after_the_first_answer() {
        let gate = gate();
        for (method, path) in [
            ("POST", "/Spawned/WebGate/users/login.xml"),
            ("POST", "/Spawned/WebGate/my_account/token.xml"),
            ("POST", "/Spawned/WebGate/my_account/game_accounts.xml"),
        ] {
            for authorization in [None, Some("Arena 00000000-0000-0000-0000-000000000000")] {
                let reply = gate.route(method, path, authorization, b"").await;
                assert_eq!(reply.status, "401 Unauthorized", "{path} {authorization:?}");
            }
        }
    }

    #[tokio::test]
    async fn the_http_framing_round_trips() {
        let gate = gate();
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("address");
        tokio::spawn(serve(listener, gate));

        let mut stream = TcpStream::connect(address).await.expect("connect");
        stream
            .write_all(
                b"GET /Spawned/WebGate/session/create.xml HTTP/1.0\r\nHost: localhost:6601\r\n\r\n",
            )
            .await
            .expect("write");
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.expect("read");
        let response = String::from_utf8(response).expect("utf8");
        assert!(response.starts_with("HTTP/1.0 200 OK\r\n"), "{response}");
        assert!(
            response.contains("Content-Type: application/xml\r\n"),
            "{response}"
        );
        assert!(response.contains("<Session>"), "{response}");

        // And the gate is still serving after that connection closed.
        let mut stream = TcpStream::connect(address).await.expect("connect again");
        stream
            .write_all(b"GET /Spawned/WebGate/session/create.xml HTTP/1.0\r\n\r\n")
            .await
            .expect("write");
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.expect("read");
        assert!(String::from_utf8_lossy(&response).starts_with("HTTP/1.0 200 OK\r\n"));
    }
}
