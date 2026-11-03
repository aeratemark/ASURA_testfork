//! The supervisor's own proof, without the game and without the database: a child that keeps
//! failing is reported FAILED after the retry cap (D1.3), and a healthy child is listed by
//! `status`, stopped by `quit`, and its port freed (the stop rule).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_asura-supervisor")
}

/// The supervisor checks the database before it starts anything, so these tests are live like the
/// repository's other database tests: they run when one is configured and skip cleanly when not.
fn database_configured() -> bool {
    if std::env::var_os("ASURA_DATABASE_URL").is_some() {
        return true;
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::fs::read_to_string(root.join(".env"))
        .map(|text| text.contains("ASURA_DATABASE_URL"))
        .unwrap_or(false)
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// The supervisor registers what it starts (S2); the test child's row is removed when the harness
/// goes away, so a test run leaves the live registry as it found it.
fn clean_registry_row() {
    let Some(url) = asura_db::config::database_url() else {
        return;
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime");
    runtime.block_on(async {
        if let Ok(pool) = asura_db::PgPool::connect(&url).await {
            let _ = asura_db::catalog::delete(&pool, "camtest", "camtest").await;
        }
    });
}

struct Harness {
    dir: PathBuf,
    supervisor: Child,
    stderr: mpsc::Receiver<String>,
}

impl Harness {
    fn new(exit_after_ms: u64, restart_max: u32, with_health: bool) -> Harness {
        let dir = std::env::temp_dir().join(format!(
            "asura-supervisor-it-{}-{}",
            std::process::id(),
            free_port()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        clean_registry_row();
        let listen = free_port();
        let control = free_port();
        let health = if with_health {
            format!("health = {{ tcp = \"127.0.0.1:{listen}\" }}\n")
        } else {
            String::new()
        };
        let text = format!(
            "\
version = 1

[settings]
control = \"127.0.0.1:{control}\"
health_timeout_secs = 10

[services.camtest]
exe = '{bin}'
args = [\"--selftest-child\", \"--listen\", \"127.0.0.1:{listen}\", \"--exit-after-ms\", \"{exit_after_ms}\", \"--exit-code\", \"7\"]
port = {listen}
{health}log = \"logs/camtest.log\"
restart = \"on-failure\"
restart_max = {restart_max}
restart_delay_ms = 100
",
            bin = binary(),
            exit_after_ms = exit_after_ms,
            listen = listen,
            health = health,
            restart_max = restart_max,
        );
        let path = dir.join("services.toml");
        std::fs::write(&path, text).expect("write config");
        let mut supervisor = Command::new(binary())
            .args(["start", "--config", path.to_str().unwrap()])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("supervisor starts");
        let stderr = supervisor.stderr.take().expect("piped stderr");
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        Harness {
            dir,
            supervisor,
            stderr: rx,
        }
    }

    fn wait_for(&self, needle: &str, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            match self.stderr.recv_timeout(left) {
                Ok(line) => {
                    eprintln!("supervisor: {line}");
                    if line.contains(needle) {
                        return true;
                    }
                }
                Err(_) => return false,
            }
        }
    }

    fn config_value(&self, key: &str) -> String {
        let text = std::fs::read_to_string(self.dir.join("services.toml")).expect("config");
        let line = text
            .lines()
            .find(|line| line.trim_start().starts_with(&format!("{key} =")))
            .expect("config line");
        let value = line.split_once('=').expect("key = value").1.trim();
        value.trim_matches('"').to_string()
    }

    fn control(&self, command: &str) -> String {
        let address: std::net::SocketAddr = self
            .config_value("control")
            .parse()
            .expect("control address");
        let mut stream = TcpStream::connect(address).expect("control channel");
        stream
            .write_all(format!("{command}\n").as_bytes())
            .expect("write");
        let mut reply = String::new();
        stream.read_to_string(&mut reply).expect("read");
        reply
    }

    fn listen_port(&self) -> u16 {
        self.config_value("port").parse().expect("port")
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = self.supervisor.kill();
        let _ = self.supervisor.wait();
        clean_registry_row();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn a_child_that_keeps_failing_is_reported_after_the_retry_cap() {
    if !database_configured() {
        eprintln!("skipped: no database configured");
        return;
    }
    let harness = Harness::new(40, 2, false);
    assert!(
        harness.wait_for("FAILED", Duration::from_secs(20)),
        "the supervisor must report FAILED after the retry cap, never restart forever"
    );
    let status = harness.control("status");
    eprintln!("status:\n{status}");
    assert!(status.contains("camtest"), "status: {status}");
    assert!(status.contains("failed"), "status: {status}");
}

#[test]
fn a_healthy_child_is_ready_listed_stopped_and_its_port_freed() {
    if !database_configured() {
        eprintln!("skipped: no database configured");
        return;
    }
    let mut harness = Harness::new(600_000, 3, true);
    assert!(
        harness.wait_for("ready", Duration::from_secs(20)),
        "the child should become ready"
    );
    let status = harness.control("status");
    eprintln!("status:\n{status}");
    assert!(
        status.contains("camtest") && status.contains("ready"),
        "status: {status}"
    );
    let reply = harness.control("quit");
    assert_eq!(reply.trim(), "ok", "reply: {reply:?}");
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if harness.supervisor.try_wait().expect("try_wait").is_some() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the supervisor did not exit after quit"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    let listen = harness.listen_port();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if TcpListener::bind(("127.0.0.1", listen)).is_ok() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the listen port {listen} is still in use after quit"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}
