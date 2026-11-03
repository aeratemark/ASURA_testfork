//! The auth service's entry point: configuration, startup order, and the listener.
//!
//! The startup order is the sequence a restart has to follow (`docs/02-architecture.md`, "Startup
//! order"), and it is enforced here rather than described:
//!
//! 1. read the configuration (the database URL, the bind address);
//! 2. open the database pool;
//! 3. **refuse to run against a database that is older than this build** (`asura_db::migrations`);
//! 4. bind the endpoint and accept logins.
//!
//! Step 3 is the one worth keeping: a service that starts against a stale schema fails later on some
//! unrelated query, saying nothing about the cause.

use std::net::SocketAddr;
use std::process::ExitCode;

use asura_auth::{serve, Trace};
use asura_db::migrations::{self, EnsureError};

const DEFAULT_BIND: &str = "127.0.0.1:6112";

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|arg| arg == "--version" || arg == "-V") {
        println!("asura-auth {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_help();
        return ExitCode::SUCCESS;
    }

    let bind = match argument(&args, "--bind") {
        Some(value) => match value.parse::<SocketAddr>() {
            Ok(address) => address,
            Err(err) => {
                eprintln!("asura-auth: --bind {value:?} is not an address: {err}");
                return ExitCode::from(2);
            }
        },
        None => match std::env::var("ASURA_AUTH_BIND") {
            Ok(value) if !value.trim().is_empty() => match value.trim().parse() {
                Ok(address) => address,
                Err(err) => {
                    eprintln!("asura-auth: ASURA_AUTH_BIND {value:?} is not an address: {err}");
                    return ExitCode::from(2);
                }
            },
            _ => DEFAULT_BIND.parse().expect("the default address parses"),
        },
    };

    let Some(database_url) = asura_db::config::database_url() else {
        eprintln!(
            "asura-auth: no {}. Set it, or copy .env.example to .env and run tools/db-setup.ps1.",
            asura_db::config::DATABASE_URL_VARIABLE
        );
        return ExitCode::from(2);
    };

    eprintln!("asura-auth {} starting", env!("CARGO_PKG_VERSION"));
    eprintln!("asura-auth: database ready, connecting");

    let pool = match asura_db::placement::connect(&database_url, 8).await {
        Ok(pool) => pool,
        Err(err) => {
            eprintln!("asura-auth: could not connect to the database: {err}");
            return ExitCode::FAILURE;
        }
    };

    match migrations::ensure_current(&pool).await {
        Ok(()) => {}
        Err(EnsureError::TooOld(too_old)) => {
            eprintln!("asura-auth: {too_old}");
            eprintln!("asura-auth: refusing to start against a stale schema");
            return ExitCode::from(3);
        }
        Err(EnsureError::Database(err)) => {
            eprintln!("asura-auth: could not read the schema version: {err}");
            return ExitCode::FAILURE;
        }
    }

    let listener = match tokio::net::TcpListener::bind(bind).await {
        Ok(listener) => listener,
        Err(err) => {
            eprintln!("asura-auth: could not bind {bind}: {err}");
            return ExitCode::FAILURE;
        }
    };

    if let Err(err) = serve(listener, pool, Trace::from_environment()).await {
        eprintln!("asura-auth: the listener stopped: {err}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

/// The value after `name`, if the flag was given.
fn argument(args: &[String], name: &str) -> Option<String> {
    let index = args.iter().position(|arg| arg == name)?;
    args.get(index + 1).cloned()
}

fn print_help() {
    println!(
        "asura-auth {} - the ASURA auth service",
        env!("CARGO_PKG_VERSION")
    );
    println!();
    println!("USAGE:");
    println!("    asura-auth [--bind ADDRESS]");
    println!();
    println!("OPTIONS:");
    println!("    --bind ADDRESS   listen here (default {DEFAULT_BIND}, or ASURA_AUTH_BIND)");
    println!("    -V, --version    print the version and exit");
    println!("    -h, --help       print this and exit");
    println!();
    println!("ENVIRONMENT:");
    println!("    ASURA_DATABASE_URL   the PostgreSQL URL (falls back to the repository .env)");
    println!("    ASURA_AUTH_BIND      the default bind address");
    println!("    ASURA_TRACE=1        log every message: direction, header, size, first bytes");
    println!();
    println!("The trace is what answers \"why does the client say 007\" - turn it on for any real");
    println!("client session.");
}
