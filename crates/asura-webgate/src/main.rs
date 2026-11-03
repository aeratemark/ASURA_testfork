//! `asura-webgate` -- the portal web gate the client asks who it is before it logs in.
//!
//! The reference ran this as `tools/webgate.py` on port 6601. This is the same service in this
//! workspace: four `/Spawned/WebGate/...` endpoints, answered from the database the auth service
//! reads, so the identity the client is handed is the identity the login will resolve.
//!
//! With `-portal 127.0.0.1` and nothing listening on 6601 the client opens the auth connection,
//! sends its preamble, gets its session salt and closes without ever logging in -- which looks
//! exactly like "the server is not running" even when it is. So this gate is not optional for a
//! `-portal` client.
//!
//! Usage: `asura-webgate [--bind ADDRESS]`, default `127.0.0.1:6601` (the port the client calls).

use std::process::ExitCode;

const DEFAULT_BIND: &str = "127.0.0.1:6601";

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|arg| arg == "-h" || arg == "--help") {
        print_help();
        return ExitCode::SUCCESS;
    }
    if args.iter().any(|arg| arg == "-V" || arg == "--version") {
        println!("asura-webgate {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }

    let bind = match argument(&args, "--bind") {
        Some(value) => match asura_webgate::parse_bind(&value) {
            Ok(bind) => bind,
            Err(err) => {
                eprintln!("asura-webgate: {err}");
                return ExitCode::from(2);
            }
        },
        None => match std::env::var("ASURA_GATE_BIND") {
            Ok(value) => match asura_webgate::parse_bind(&value) {
                Ok(bind) => bind,
                Err(err) => {
                    eprintln!("asura-webgate: ASURA_GATE_BIND {err}");
                    return ExitCode::from(2);
                }
            },
            Err(_) => DEFAULT_BIND.parse().expect("the default address parses"),
        },
    };

    let Some(database_url) = asura_db::config::database_url() else {
        eprintln!(
            "asura-webgate: no {}. Set it, or copy .env.example to .env and run tools/db-setup.ps1.",
            asura_db::config::DATABASE_URL_VARIABLE
        );
        return ExitCode::from(2);
    };

    eprintln!("asura-webgate {} starting", env!("CARGO_PKG_VERSION"));
    eprintln!("asura-webgate: database ready, connecting");

    let pool = match asura_db::placement::connect(&database_url, 4).await {
        Ok(pool) => pool,
        Err(err) => {
            eprintln!("asura-webgate: could not connect to the database: {err}");
            return ExitCode::FAILURE;
        }
    };

    let listener = match tokio::net::TcpListener::bind(bind).await {
        Ok(listener) => listener,
        Err(err) => {
            eprintln!("asura-webgate: cannot listen on {bind}: {err}");
            return ExitCode::FAILURE;
        }
    };
    eprintln!(
        "asura-webgate: listening on {} -- the client asks it who it is before it logs in",
        listener
            .local_addr()
            .map(|address| address.to_string())
            .unwrap_or_else(|_| bind.to_string())
    );

    if let Err(err) = asura_webgate::serve(listener, asura_webgate::Gate::database(pool)).await {
        eprintln!("asura-webgate: the listener stopped: {err}");
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
        "asura-webgate {} -- the portal web gate",
        env!("CARGO_PKG_VERSION")
    );
    println!();
    println!("USAGE:");
    println!("    asura-webgate [--bind ADDRESS]");
    println!();
    println!("OPTIONS:");
    println!("    --bind ADDRESS    listen here (default {DEFAULT_BIND}, or ASURA_GATE_BIND)");
    println!("                      this is the port a client launched with -portal calls");
    println!("    -V, --version     print the version and exit");
    println!("    -h, --help        print this and exit");
    println!();
    println!("ENVIRONMENT:");
    println!("    ASURA_DATABASE_URL    the PostgreSQL URL (falls back to the repository .env)");
}
