//! `asura-supervisor`: starts this machine's services, watches them, and reports.
//!
//! The list is `config/services.toml` (`docs/16-implementation-plan.md`, S1): what to run, with what
//! arguments, in what order, and where each one logs. The database is checked and waited for, never
//! started. A service that keeps crashing is retried a bounded number of times and then reported
//! FAILED - the supervisor never restarts one forever (D1.3).
//!
//! Commands:
//!
//! ```text
//! asura-supervisor start                 # build if needed, seed, start, watch; Ctrl+C stops
//! asura-supervisor status                # what is running, ready, restarting, or failed
//! asura-supervisor stop [--only NAME]    # stop one service, or everything
//! asura-supervisor restart NAME          # stop and start one service
//! asura-supervisor build                 # build the services only (start does this too)
//! asura-supervisor schema                # apply the database migrations (psql, any OS)
//! asura-supervisor seed                  # write the dev identities and the map catalogue (psql)
//! ```
//!
//! Every command takes `--config FILE` (default: `config/services.toml`).

mod config;
mod control;
mod health;
mod process;
mod psql;
mod registry;
mod status;
mod supervise;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use asura_db::PgPool;
use tokio::net::TcpListener;

use supervise::Supervisor;

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--selftest-child") {
        return selftest_child(&args).await;
    }
    if args.iter().any(|arg| arg == "--version" || arg == "-V") {
        println!("asura-supervisor {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    match args.first().map(String::as_str) {
        Some("build") => finish(build_services(None)),
        Some("start") => start(&args).await,
        Some("stop") => stop(&args).await,
        Some("restart") => restart(&args).await,
        Some("status") => status(&args).await,
        Some("deploy-world") => deploy_world_cli(&args).await,
        Some("schema") => finish(schema()),
        Some("seed") => finish(seed_command()),
        Some("--help") | Some("-h") => {
            print_help();
            ExitCode::SUCCESS
        }
        None => {
            print_help();
            ExitCode::from(2)
        }
        Some(other) => {
            eprintln!("asura-supervisor: unknown command {other:?}");
            print_help();
            ExitCode::from(2)
        }
    }
}

/// Start everything listed - build if needed, seed, then start - and watch it until Ctrl+C or a
/// `stop`/`quit` command.
async fn start(args: &[String]) -> ExitCode {
    let path = config_path(args);
    let config = match config::load(&path) {
        Ok(config) => config,
        Err(err) => {
            eprintln!("asura-supervisor: {err}");
            return ExitCode::from(2);
        }
    };
    let control = config.settings.control.clone();
    let address: SocketAddr = match control.parse() {
        Ok(address) => address,
        Err(err) => {
            eprintln!("asura-supervisor: control {control:?} is not an address: {err}");
            return ExitCode::from(2);
        }
    };
    let listener = match TcpListener::bind(address).await {
        Ok(listener) => listener,
        Err(err) => {
            eprintln!(
                "asura-supervisor: cannot listen on {address} ({err}); another supervisor is \
                 probably running (stop it first: `asura-supervisor stop`)"
            );
            return ExitCode::from(3);
        }
    };
    let pool = match supervise::connect_database().await {
        Ok(pool) => pool,
        Err(err) => {
            eprintln!("asura-supervisor: {err}");
            return ExitCode::from(2);
        }
    };
    // `start` stays one command, like the old start script: build what the list runs (fast when
    // up to date) and seed the rows a fresh database needs, then start and watch.
    if let Err(err) = build_services(Some(&config)) {
        eprintln!("asura-supervisor: {err}");
        return ExitCode::from(1);
    }
    if let Err(err) = seed() {
        eprintln!(
            "asura-supervisor: seeding failed: {err}; continuing (the rows may already be present)"
        );
    }
    let shared = Supervisor::new(config, Some(pool));
    let quitting = shared.lock().unwrap().quit.clone();
    eprintln!("asura-supervisor: starting from {}", path.display());
    let shared_for_control = shared.clone();
    tokio::spawn(control::serve(shared_for_control, listener));
    // The service registry (S2): every service this supervisor starts is published to the database,
    // so consumers read `service` instead of hard-coded addresses. The publish is the heartbeat.
    let registry_shared = shared.clone();
    tokio::spawn(async move {
        loop {
            if registry_shared.lock().unwrap().quit.load(Ordering::SeqCst) {
                break;
            }
            if let Err(err) = registry::publish(&registry_shared).await {
                eprintln!("asura-supervisor: registry: {err}");
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });
    supervise::start_all(&shared).await;
    {
        let sup = shared.lock().unwrap();
        for (name, service) in &sup.services {
            eprintln!(
                "  {name}: {} (pid {})",
                service.state.as_str(),
                service
                    .pid
                    .map(|pid| pid.to_string())
                    .unwrap_or_else(|| "-".to_string())
            );
        }
    }
    eprintln!(
        "asura-supervisor: running; control on {address}; Ctrl+C or `asura-supervisor stop` stops it"
    );
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = sigterm() => {}
        _ = wait_for(&quitting) => {}
    }
    eprintln!("asura-supervisor: stopping");
    supervise::stop_all(&shared, Duration::from_secs(15)).await;
    let _ = registry::publish(&shared).await; // the rows say "stopped"
    ExitCode::SUCCESS
}

/// A graceful stop from a container runtime: `docker stop` sends SIGTERM, and the supervisor must
/// treat it as the same quit as Ctrl+C, so the services are stopped and the registry rows are
/// marked `stopped` instead of being killed blind (`docker/`, docs/19-running.md).
#[cfg(unix)]
async fn sigterm() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut term = signal(SignalKind::terminate()).expect("a SIGTERM handler can be installed");
    term.recv().await;
}

#[cfg(not(unix))]
async fn sigterm() {
    std::future::pending::<()>().await;
}

async fn wait_for(flag: &AtomicBool) {
    loop {
        if flag.load(Ordering::SeqCst) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Stop one service (`--only`), or everything (and the supervisor).
async fn stop(args: &[String]) -> ExitCode {
    let path = config_path(args);
    let config = match config::load(&path) {
        Ok(config) => config,
        Err(err) => {
            eprintln!("asura-supervisor: {err}");
            return ExitCode::from(2);
        }
    };
    let control = config.settings.control.clone();
    let command = match argument(args, "--only") {
        Some(name) => format!("stop {name}"),
        None => "quit".to_string(),
    };
    match control::request(&control, &command).await {
        Ok(reply) => {
            println!("{reply}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("asura-supervisor: {err}");
            ExitCode::from(3)
        }
    }
}

async fn restart(args: &[String]) -> ExitCode {
    let Some(name) = args.get(1).filter(|name| !name.starts_with('-')).cloned() else {
        eprintln!("usage: asura-supervisor restart <service> [--config FILE]");
        return ExitCode::from(2);
    };
    let path = config_path(args);
    let config = match config::load(&path) {
        Ok(config) => config,
        Err(err) => {
            eprintln!("asura-supervisor: {err}");
            return ExitCode::from(2);
        }
    };
    let control = config.settings.control.clone();
    match control::request(&control, &format!("restart {name}")).await {
        Ok(reply) => {
            println!("{reply}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("asura-supervisor: {err}");
            ExitCode::from(3)
        }
    }
}

async fn status(args: &[String]) -> ExitCode {
    let path = config_path(args);
    let config = match config::load(&path) {
        Ok(config) => config,
        Err(err) => {
            eprintln!("asura-supervisor: {err}");
            return ExitCode::from(2);
        }
    };
    let control = config.settings.control.clone();
    match control::request(&control, "status").await {
        Ok(reply) => {
            println!("{reply}");
            print_registry().await;
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("asura-supervisor: {err}");
            ExitCode::from(3)
        }
    }
}

/// The service registry (S2), read from the database: every service every supervisor and world has
/// announced, which the local process table cannot show.
async fn print_registry() {
    let Some(url) = asura_db::config::database_url() else {
        println!("\nregistry: no database configured (ASURA_DATABASE_URL)");
        return;
    };
    let pool = match tokio::time::timeout(Duration::from_secs(3), PgPool::connect(&url)).await {
        Ok(Ok(pool)) => pool,
        Ok(Err(err)) => {
            println!("\nregistry: cannot read the database ({err})");
            return;
        }
        Err(_) => {
            println!("\nregistry: the database did not answer");
            return;
        }
    };
    match asura_db::catalog::services(&pool, 60.0).await {
        Ok(rows) if rows.is_empty() => println!("\nregistry: empty"),
        Ok(rows) => {
            println!("\nregistry (service table; STALE = heartbeat older than 60 s):");
            for row in rows {
                println!(
                    "  {:<10} {:<16} {:<22} {:<8} {:<8} {}",
                    row.kind,
                    row.name,
                    row.address,
                    row.port,
                    row.state,
                    if row.fresh {
                        "fresh".to_string()
                    } else {
                        format!("STALE ({} s)", row.age_secs.round() as i64)
                    }
                );
            }
        }
        Err(err) => println!("\nregistry: {err}"),
    }
}

/// Build the three services the list starts.
fn build_services(fallback: Option<&config::Config>) -> Result<(), String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let tried = std::process::Command::new(cargo)
        .args([
            "build",
            "--offline",
            "-p",
            "asura-auth",
            "-p",
            "asura-world",
            "-p",
            "asura-webgate",
        ])
        .current_dir(config::repo_root())
        .status();
    match tried {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(format!("the build failed (exit {status})")),
        // Cargo could not be run at all: a direct binary invocation with no cargo on PATH is fine
        // as long as every program the list names is already present.
        Err(err) => match fallback {
            Some(config)
                if config
                    .services
                    .values()
                    .all(|service| config::resolve_exe(&service.exe).is_ok()) =>
            {
                eprintln!(
                    "asura-supervisor: cargo could not be run ({err}); using the binaries that are present"
                );
                Ok(())
            }
            _ => Err(format!(
                "cannot run cargo ({err}); build once with `asura-supervisor build`"
            )),
        },
    }
}

/// The rows a fresh database needs - the two steps the old start script ran before starting the
/// services. Cross-platform: the SQL is the repository's own data files, applied by `psql`.
fn seed() -> Result<(), String> {
    let psql = psql::find()?;
    let url = database_url()?;
    let tools = config::repo_root().join("tools");
    psql::apply_file(&psql, &url, &tools.join("seed-dev.sql"))?;
    eprintln!("asura-supervisor: seeded the dev identities (tools/seed-dev.sql)");
    psql::apply_file(&psql, &url, &tools.join("map-config.sql"))?;
    eprintln!("asura-supervisor: seeded the maps (tools/map-config.sql)");
    Ok(())
}

/// `asura-supervisor seed`: the rows a fresh database needs, then the pairs the client presents.
fn seed_command() -> Result<(), String> {
    seed()?;
    println!("The rig's portal DLL presents these pairs (account, portal user id, session id):");
    println!("  user1@example.com  fa520ee2-4419-4eb4-ae49-6e9abe6ef24f  d8b9bf5d-90b1-4cbd-9b76-88da7be763b6");
    println!("  user2@example.com  5fb222bc-9d19-4308-ac3e-3c62685bc6ae  224aee30-d8a5-4f18-ad8e-b5fcbf121c4e");
    println!("  user3@example.com  9957c79c-b93c-4b60-9d84-9f38a1f62be3  ccdb4b8d-1c88-48c0-bda0-0a5e8d87f4da");
    Ok(())
}

/// `asura-supervisor schema`: apply every migration, in order. Cross-platform: the migrations are
/// SQL files.
fn schema() -> Result<(), String> {
    let psql = psql::find()?;
    let url = database_url()?;
    let directory = config::repo_root()
        .join("crates")
        .join("asura-db")
        .join("migrations");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&directory)
        .map_err(|err| format!("cannot read {}: {err}", directory.display()))?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().map(|ext| ext == "sql").unwrap_or(false))
        .collect();
    files.sort();
    if files.is_empty() {
        return Err(format!("no migrations found in {}", directory.display()));
    }
    for file in &files {
        psql::apply_file(&psql, &url, file)?;
        eprintln!(
            "asura-supervisor: applied {}",
            file.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default()
        );
    }
    Ok(())
}

fn database_url() -> Result<String, String> {
    asura_db::config::database_url().ok_or_else(|| {
        "ASURA_DATABASE_URL is not configured (copy .env.example to .env)".to_string()
    })
}

/// The rolling deploy (S3): `deploy-world --old <world id> --new <world id>`.
async fn deploy_world_cli(args: &[String]) -> ExitCode {
    let (Some(old), Some(new)) = (argument(args, "--old"), argument(args, "--new")) else {
        eprintln!("usage: asura-supervisor deploy-world --old <world id> --new <world id>");
        return ExitCode::from(2);
    };
    let path = config_path(args);
    let config = match config::load(&path) {
        Ok(config) => config,
        Err(err) => {
            eprintln!("asura-supervisor: {err}");
            return ExitCode::from(2);
        }
    };
    let control = config.settings.control.clone();
    match control::request(&control, &format!("deploy-world {old} {new}")).await {
        Ok(reply) => {
            println!("{reply}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("asura-supervisor: {err}");
            ExitCode::from(3)
        }
    }
}

/// The test child: a port that stays bound for a while, and an exit code the tests choose. It is
/// how the supervisor's restart rule is exercised without the game.
async fn selftest_child(args: &[String]) -> ExitCode {
    let listen = argument(args, "--listen").unwrap_or_else(|| "127.0.0.1:0".to_string());
    let exit_after_ms: u64 = argument(args, "--exit-after-ms")
        .and_then(|value| value.parse().ok())
        .unwrap_or(60_000);
    let exit_code: i32 = argument(args, "--exit-code")
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let listener = match TcpListener::bind(&listen).await {
        Ok(listener) => listener,
        Err(err) => {
            eprintln!("selftest-child: cannot listen on {listen}: {err}");
            return ExitCode::from(3);
        }
    };
    let deadline = tokio::time::Instant::now() + Duration::from_millis(exit_after_ms);
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            break;
        }
        if tokio::time::timeout(left, listener.accept()).await.is_err() {
            break;
        }
    }
    ExitCode::from(exit_code as u8)
}

fn finish(result: Result<(), String>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("asura-supervisor: {err}");
            ExitCode::from(1)
        }
    }
}

fn argument(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|index| args.get(index + 1))
        .cloned()
}

fn config_path(args: &[String]) -> PathBuf {
    argument(args, "--config")
        .map(PathBuf::from)
        .unwrap_or_else(config::default_path)
}

fn print_help() {
    println!(
        "asura-supervisor {} - starts this machine's services and watches them.",
        env!("CARGO_PKG_VERSION")
    );
    println!();
    println!("  asura-supervisor build                build the services (start does this too)");
    println!("  asura-supervisor schema               apply the database migrations");
    println!(
        "  asura-supervisor seed                 write the dev identities and the map catalogue"
    );
    println!(
        "  asura-supervisor start                build if needed, seed, start, watch; Ctrl+C stops"
    );
    println!(
        "  asura-supervisor status               what is running, ready, restarting, or failed"
    );
    println!("  asura-supervisor stop [--only NAME]   stop one service, or everything");
    println!("  asura-supervisor restart NAME         stop and start one service");
    println!(
        "  asura-supervisor deploy-world --old N --new M   rolling deploy: start the new build, drain the old one"
    );
    println!();
    println!("  Every command takes --config FILE (default: config/services.toml).");
}
