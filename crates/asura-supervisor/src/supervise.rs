//! The supervisor's runtime: one child per service, a watcher per child, and the bounded restart
//! rule (D1.3: restart a few times, then report the service dead - never forever).
//!
//! State lives behind one mutex and is never held across an `await`. The watcher owns its child; the
//! control channel asks a watcher to stop through a flag and a kill signal (a one-shot channel), and
//! the exit is then classified as expected ("stopped") rather than as a crash.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use asura_db::placement;
use asura_db::PgPool;
use tokio::process::Child;
use tokio::sync::mpsc;
use tokio::time::{sleep, Instant};

use crate::config::{Config, ServiceConfig};
use crate::health;
use crate::process;

/// What the supervisor knows about one service right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Starting,
    Ready,
    Restarting,
    Stopped,
    Failed,
}

impl State {
    pub fn as_str(self) -> &'static str {
        match self {
            State::Starting => "starting",
            State::Ready => "ready",
            State::Restarting => "restarting",
            State::Stopped => "stopped",
            State::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Service {
    pub state: State,
    pub pid: Option<u32>,
    pub restarts: u32,
    pub last_error: Option<String>,
}

pub struct Supervisor {
    pub config: Config,
    pub pool: Option<PgPool>,
    pub services: BTreeMap<String, Service>,
    pub quit: Arc<AtomicBool>,
    stopping: HashMap<String, Arc<AtomicBool>>,
    stop_signals: HashMap<String, mpsc::Sender<()>>,
}

pub type Shared = Arc<Mutex<Supervisor>>;

impl Supervisor {
    pub fn new(config: Config, pool: Option<PgPool>) -> Shared {
        let services = config
            .services
            .keys()
            .map(|name| {
                (
                    name.clone(),
                    Service {
                        state: State::Stopped,
                        pid: None,
                        restarts: 0,
                        last_error: None,
                    },
                )
            })
            .collect();
        Arc::new(Mutex::new(Supervisor {
            config,
            pool,
            services,
            quit: Arc::new(AtomicBool::new(false)),
            stopping: HashMap::new(),
            stop_signals: HashMap::new(),
        }))
    }
}

/// The database, waited for until it answers (or ten seconds pass). It is never started here.
pub async fn connect_database() -> Result<PgPool, String> {
    let Some(url) = asura_db::config::database_url() else {
        return Err(
            "ASURA_DATABASE_URL is not configured (copy .env.example to .env, or run \
             tools/db-setup.ps1)"
                .to_string(),
        );
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match tokio::time::timeout(Duration::from_secs(3), PgPool::connect(&url)).await {
            Ok(Ok(pool)) => return Ok(pool),
            Ok(Err(err)) => {
                if Instant::now() >= deadline {
                    return Err(format!("database unreachable: {err}"));
                }
            }
            Err(_) => {
                if Instant::now() >= deadline {
                    return Err("database unreachable: timed out".into());
                }
            }
        }
        sleep(Duration::from_millis(250)).await;
    }
}

/// Start every listed service, dependencies first. One service that fails is reported and the
/// others still start, so one bad entry does not hide the rest.
pub async fn start_all(shared: &Shared) {
    let (order, health_timeout) = {
        let sup = shared.lock().unwrap();
        (
            sup.config
                .order()
                .unwrap_or_else(|_| sup.services.keys().cloned().collect()),
            Duration::from_secs(sup.config.settings.health_timeout_secs.max(1)),
        )
    };
    for name in order {
        if let Err(err) = start_service(shared, &name, health_timeout).await {
            eprintln!("{name}: FAILED to start: {err}");
        }
    }
}

/// Start one service and wait for the watcher's verdict: ready, or failed.
pub async fn start_service(
    shared: &Shared,
    name: &str,
    health_timeout: Duration,
) -> Result<(), String> {
    let (service, pool) = {
        let sup = shared.lock().unwrap();
        if sup
            .services
            .get(name)
            .map(|service| service.pid.is_some())
            .unwrap_or(false)
        {
            return Err(format!("{name}: already running"));
        }
        let Some(service) = sup.config.services.get(name).cloned() else {
            return Err(format!("{name}: not listed"));
        };
        (service, sup.pool.clone())
    };
    if service.health.is_some() && pool.is_none() {
        return Err(format!(
            "{name}: a health check needs the database, and ASURA_DATABASE_URL is not configured"
        ));
    }
    let child = match process::spawn(name, &service) {
        Ok(child) => child,
        Err(err) => {
            set(shared, name, |state| {
                state.state = State::Failed;
                state.last_error = Some(err.clone());
            });
            return Err(err);
        }
    };
    let pid = child.id();
    let (tx, rx) = mpsc::channel(1);
    let stopping = Arc::new(AtomicBool::new(false));
    {
        let mut sup = shared.lock().unwrap();
        if let Some(state) = sup.services.get_mut(name) {
            state.state = State::Starting;
            state.pid = pid;
            state.restarts = 0;
            state.last_error = None;
        }
        sup.stopping.insert(name.to_string(), stopping.clone());
        sup.stop_signals.insert(name.to_string(), tx);
    }
    eprintln!("{name}: started (pid {})", pid_text(pid));
    let shared_for_watcher = shared.clone();
    let name_for_watcher = name.to_string();
    tokio::spawn(async move {
        watch(Watched {
            shared: shared_for_watcher,
            name: name_for_watcher,
            service,
            pool,
            child,
            rx,
            health_timeout,
            stopping,
        })
        .await;
    });
    // Wait for the watcher's verdict.
    let deadline = Instant::now() + health_timeout + Duration::from_secs(1);
    loop {
        {
            let sup = shared.lock().unwrap();
            if let Some(state) = sup.services.get(name) {
                match state.state {
                    State::Ready => return Ok(()),
                    State::Failed => {
                        return Err(state
                            .last_error
                            .clone()
                            .unwrap_or_else(|| "failed to start".into()));
                    }
                    State::Stopped => return Err("stopped during start".into()),
                    _ => {}
                }
            }
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "did not become ready within {} s",
                health_timeout.as_secs()
            ));
        }
        sleep(Duration::from_millis(100)).await;
    }
}

fn pid_text(pid: Option<u32>) -> String {
    pid.map(|pid| pid.to_string())
        .unwrap_or_else(|| "?".to_string())
}

fn set(shared: &Shared, name: &str, change: impl FnOnce(&mut Service)) {
    let mut sup = shared.lock().unwrap();
    if let Some(service) = sup.services.get_mut(name) {
        change(service);
    }
}

/// The watcher's own state: one child and what it takes to look after it.
struct Watched {
    shared: Shared,
    name: String,
    service: ServiceConfig,
    pool: Option<PgPool>,
    child: Child,
    rx: mpsc::Receiver<()>,
    health_timeout: Duration,
    stopping: Arc<AtomicBool>,
}

/// The watcher: one child, one loop. It owns the child, decides on success by its exit, and
/// restarts it by policy up to `restart_max` times.
async fn watch(watched: Watched) {
    let Watched {
        shared,
        name,
        service,
        pool,
        mut child,
        mut rx,
        health_timeout,
        stopping,
    } = watched;
    // --- health: ready once the check passes; failed if it never does.
    if let Some(check) = &service.health {
        let deadline = Instant::now() + health_timeout;
        loop {
            if health::check(check, pool.as_ref()).await {
                break;
            }
            if Instant::now() >= deadline {
                let _ = child.kill().await;
                let _ = child.wait().await;
                set(&shared, &name, |state| {
                    state.state = State::Failed;
                    state.pid = None;
                    state.last_error = Some(format!(
                        "health check did not pass within {} s",
                        health_timeout.as_secs()
                    ));
                });
                eprintln!(
                    "{name}: FAILED: health check did not pass within {} s",
                    health_timeout.as_secs()
                );
                return;
            }
            tokio::select! {
                _ = rx.recv() => {
                    stopping.store(true, Ordering::SeqCst);
                    let _ = child.kill().await;
                    let _ = child.wait().await;
                    set(&shared, &name, |state| {
                        state.state = State::Stopped;
                        state.pid = None;
                    });
                    return;
                }
                _ = sleep(Duration::from_millis(100)) => {}
            }
        }
        set(&shared, &name, |state| {
            state.state = State::Ready;
            state.last_error = None;
        });
        eprintln!("{name}: ready");
    } else {
        set(&shared, &name, |state| {
            state.state = State::Ready;
        });
    }

    // --- steady state: watch it die, and restart it by policy, with a cap.
    loop {
        tokio::select! {
            exit = child.wait() => {
                let code = exit.as_ref().ok().and_then(|status| status.code());
                if stopping.load(Ordering::SeqCst) {
                    set(&shared, &name, |state| {
                        state.state = State::Stopped;
                        state.pid = None;
                    });
                    return;
                }
                let policy = service.restart.as_str();
                let clean = code == Some(0);
                let restart = match policy {
                    "never" => false,
                    "always" => true,
                    _ => !clean,
                };
                if !restart {
                    set(&shared, &name, |state| {
                        state.state = State::Stopped;
                        state.pid = None;
                        state.last_error = Some(format!("exited ({code:?}); not restarted"));
                    });
                    eprintln!("{name}: exited ({code:?}); policy {policy:?} - not restarting");
                    return;
                }
                let restarts = {
                    let sup = shared.lock().unwrap();
                    sup.services
                        .get(&name)
                        .map(|service| service.restarts + 1)
                        .unwrap_or(1)
                };
                if restarts > service.restart_max {
                    set(&shared, &name, |state| {
                        state.state = State::Failed;
                        state.pid = None;
                        state.restarts = restarts;
                        state.last_error = Some(format!(
                            "exited ({code:?}) and {} restart(s) exhausted",
                            service.restart_max
                        ));
                    });
                    eprintln!(
                        "{name}: FAILED after {} restart(s); giving up ({code:?})",
                        service.restart_max
                    );
                    return;
                }
                set(&shared, &name, |state| {
                    state.state = State::Restarting;
                    state.pid = None;
                    state.restarts = restarts;
                });
                eprintln!(
                    "{name}: exited ({code:?}); restarting ({restarts}/{}) in {} ms",
                    service.restart_max, service.restart_delay_ms
                );
                let stopped = {
                    tokio::select! {
                        _ = rx.recv() => { true }
                        _ = sleep(Duration::from_millis(service.restart_delay_ms)) => { false }
                    }
                };
                if stopped {
                    stopping.store(true, Ordering::SeqCst);
                    set(&shared, &name, |state| {
                        state.state = State::Stopped;
                    });
                    return;
                }
                match process::spawn(&name, &service) {
                    Ok(new_child) => {
                        child = new_child;
                        let pid = child.id();
                        set(&shared, &name, |state| {
                            state.pid = pid;
                            state.state = State::Ready;
                        });
                        eprintln!("{name}: restarted (pid {})", pid_text(pid));
                    }
                    Err(err) => {
                        set(&shared, &name, |state| {
                            state.state = State::Failed;
                            state.last_error = Some(err.clone());
                        });
                        eprintln!("{name}: FAILED: {err}");
                        return;
                    }
                }
            }
            _ = rx.recv() => {
                stopping.store(true, Ordering::SeqCst);
                let _ = child.kill().await;
            }
        }
    }
}

/// Stop one service: ask its watcher to kill the child, wait for it to be gone, and wait for its
/// port to be free. Idempotent: stopping a stopped service succeeds.
pub async fn stop_service(shared: &Shared, name: &str, timeout: Duration) -> Result<(), String> {
    let (flag, signal, running) = {
        let sup = shared.lock().unwrap();
        let Some(service) = sup.services.get(name) else {
            return Err(format!("{name}: not listed"));
        };
        let running = service.pid.is_some()
            || matches!(
                service.state,
                State::Starting | State::Ready | State::Restarting
            );
        (
            sup.stopping.get(name).cloned(),
            sup.stop_signals.get(name).cloned(),
            running,
        )
    };
    if !running {
        return Ok(());
    }
    if let Some(flag) = flag {
        flag.store(true, Ordering::SeqCst);
    }
    if let Some(signal) = signal {
        let _ = signal.try_send(());
    }
    let deadline = Instant::now() + timeout;
    loop {
        {
            let sup = shared.lock().unwrap();
            if let Some(service) = sup.services.get(name) {
                if matches!(service.state, State::Stopped | State::Failed) {
                    break;
                }
            } else {
                break;
            }
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "{name}: did not stop within {} s",
                timeout.as_secs()
            ));
        }
        sleep(Duration::from_millis(50)).await;
    }
    let port = {
        let sup = shared.lock().unwrap();
        sup.config
            .services
            .get(name)
            .map(|service| service.port)
            .unwrap_or(0)
    };
    wait_port_free(port, Duration::from_secs(10)).await
}

/// Nothing may bind a service's port until the previous owner has let go of it.
pub async fn wait_port_free(port: u16, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        if std::net::TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!("port {port} is still in use"));
        }
        sleep(Duration::from_millis(100)).await;
    }
}

/// Stop one service and start it again.
pub async fn restart_service(shared: &Shared, name: &str, timeout: Duration) -> Result<(), String> {
    stop_service(shared, name, timeout).await?;
    let health_timeout = {
        let sup = shared.lock().unwrap();
        Duration::from_secs(sup.config.settings.health_timeout_secs.max(1))
    };
    start_service(shared, name, health_timeout).await
}

/// Stop everything, in reverse start order.
pub async fn stop_all(shared: &Shared, timeout: Duration) {
    let order = {
        let sup = shared.lock().unwrap();
        sup.config
            .order()
            .unwrap_or_else(|_| sup.services.keys().cloned().collect())
    };
    for name in order.into_iter().rev() {
        if let Err(err) = stop_service(shared, &name, timeout).await {
            eprintln!("{err}");
        }
    }
}

/// The rolling deploy (S3): start the new world, wait until it is registered, then mark the old
/// world draining.
///
/// The order is the point. The new world must be answering *before* the old one is told to drain -
/// the one rule - and the old world is never killed: it keeps the players already inside it and
/// leaves the registry itself once its last player has disconnected.
pub async fn deploy_world(shared: &Shared, old_world: i32, new_world: i32) -> Result<(), String> {
    let (new_service, pool) = {
        let sup = shared.lock().unwrap();
        let old = service_for_world(&sup.config, old_world)
            .ok_or_else(|| format!("no listed service hosts world {old_world}"))?;
        let new = service_for_world(&sup.config, new_world)
            .ok_or_else(|| format!("no listed service hosts world {new_world}"))?;
        if old == new {
            return Err("the old and the new world are the same service entry".into());
        }
        let pool = sup
            .pool
            .clone()
            .ok_or_else(|| "the database is not configured".to_string())?;
        (new, pool)
    };
    let health_timeout = {
        let sup = shared.lock().unwrap();
        Duration::from_secs(sup.config.settings.health_timeout_secs.max(1))
    };
    // The new world first: a travel is only ever answered by a world that is already answering. A
    // config that already lists both worlds has the new one running already; the deploy then only
    // has to prove it is registered before the old one is drained.
    let already_running = {
        let sup = shared.lock().unwrap();
        sup.services
            .get(&new_service)
            .map(|service| service.pid.is_some())
            .unwrap_or(false)
    };
    if !already_running {
        start_service(shared, &new_service, health_timeout).await?;
    }
    let deadline = Instant::now() + health_timeout + Duration::from_secs(1);
    loop {
        let registered = placement::world_endpoint(&pool, new_world, 60.0)
            .await
            .map(|endpoint| endpoint.map(|endpoint| endpoint.fresh).unwrap_or(false))
            .unwrap_or(false);
        if registered {
            break;
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "world {new_world} started but has not registered yet; the old world was not drained"
            ));
        }
        sleep(Duration::from_millis(200)).await;
    }
    // Only now is the old world told to drain; it finishes its players and leaves by itself.
    placement::set_draining(&pool, old_world, true)
        .await
        .map_err(|err| format!("cannot mark world {old_world} draining: {err}"))?;
    Ok(())
}

/// The service in the list whose arguments name this world (`--world <id>`).
fn service_for_world(config: &crate::config::Config, world_id: i32) -> Option<String> {
    config.services.iter().find_map(|(name, service)| {
        let mut arguments = service.args.iter();
        while let Some(argument) = arguments.next() {
            if argument == "--world"
                && arguments.next().and_then(|value| value.parse::<i32>().ok()) == Some(world_id)
            {
                return Some(name.clone());
            }
        }
        None
    })
}
