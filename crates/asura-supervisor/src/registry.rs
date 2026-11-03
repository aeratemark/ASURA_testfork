//! The supervisor's side of the service registry (S2): every service it starts gets a row in
//! `service`, refreshed every few seconds so consumers can find it and see it is alive.
//!
//! The kind and address come from the configuration; the state comes from the runtime. The rows
//! are an upsert, so publishing repeatedly is the heartbeat.

use asura_db::catalog;

use crate::config::ServiceConfig;
use crate::supervise::{Shared, State};

/// Which kind a listed service is: the config's `kind`, else the name before the first `-`
/// (`world-8001` -> `world`), else the name itself.
pub fn kind_of(name: &str, configured: &Option<String>) -> String {
    if let Some(kind) = configured {
        let kind = kind.trim();
        if !kind.is_empty() {
            return kind.to_string();
        }
    }
    name.split('-').next().unwrap_or(name).to_string()
}

/// The address a service is registered at: what it binds (`--bind host:port`), else loopback.
pub fn address_of(service: &ServiceConfig) -> String {
    for window in service.args.windows(2) {
        if window[0] == "--bind" {
            if let Some((host, _port)) = window[1].rsplit_once(':') {
                if !host.is_empty() {
                    return host.to_string();
                }
            }
        }
    }
    "127.0.0.1".to_string()
}

/// How the runtime state is published.
fn state_str(state: State) -> &'static str {
    match state {
        State::Starting | State::Restarting => "starting",
        State::Ready => "ready",
        State::Stopped | State::Failed => "stopped",
    }
}

/// Publish every listed service's current row. One upsert per service; runs every few seconds
/// while the supervisor is up, so the rows are also the heartbeat.
pub async fn publish(shared: &Shared) -> Result<(), String> {
    let (pool, rows) = {
        let sup = shared.lock().unwrap();
        let Some(pool) = sup.pool.clone() else {
            return Err("the database is not configured".into());
        };
        let rows: Vec<(String, String, String, i32, String)> = sup
            .config
            .services
            .iter()
            .map(|(name, service)| {
                let kind = kind_of(name, &service.kind);
                let state = sup
                    .services
                    .get(name)
                    .map(|service| state_str(service.state))
                    .unwrap_or("stopped");
                (
                    kind,
                    name.clone(),
                    address_of(service),
                    i32::from(service.port),
                    state.to_string(),
                )
            })
            .collect();
        (pool, rows)
    };
    for (kind, name, address, port, state) in rows {
        catalog::upsert(&pool, &kind, &name, &address, port, &build(), &state)
            .await
            .map_err(|err| format!("cannot publish {name}: {err}"))?;
    }
    Ok(())
}

fn build() -> String {
    format!("asura/{}", env!("CARGO_PKG_VERSION"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service(args: &str) -> ServiceConfig {
        let text = format!(
            "version = 1\n\n[services.a]\nexe = \"prog\"\nport = 1\nargs = [{}]\n",
            args
        );
        crate::config::parse(&text)
            .expect("the test config parses")
            .services["a"]
            .clone()
    }

    #[test]
    fn the_kind_prefers_the_config_then_the_name_prefix() {
        assert_eq!(kind_of("webgate", &Some("webgate".into())), "webgate");
        assert_eq!(kind_of("anything", &Some("  world  ".into())), "world");
        assert_eq!(kind_of("world-8001", &None), "world");
        assert_eq!(kind_of("auth", &None), "auth");
        assert_eq!(kind_of("no-dash", &None), "no");
    }

    #[test]
    fn the_address_is_what_the_service_binds() {
        assert_eq!(
            address_of(&service("\"--bind\", \"10.0.0.5:6112\"")),
            "10.0.0.5"
        );
        assert_eq!(address_of(&service("\"--world\", \"8001\"")), "127.0.0.1");
        assert_eq!(
            address_of(&service("\"--bind\", \"127.0.0.1:6601\"")),
            "127.0.0.1"
        );
    }
}
