//! "Is it alive?" - the two checks the supervisor knows.

use std::time::Duration;

use tokio::net::TcpStream;

use asura_db::placement;
use asura_db::PgPool;

use crate::config::Health;

/// One check, now.
pub async fn check(health: &Health, pool: Option<&PgPool>) -> bool {
    match health {
        Health::Tcp { tcp } => matches!(
            tokio::time::timeout(Duration::from_secs(1), TcpStream::connect(tcp)).await,
            Ok(Ok(_))
        ),
        Health::World { world } => match pool {
            Some(pool) => placement::world_endpoint(pool, *world, 60.0)
                .await
                .map(|endpoint| endpoint.map(|endpoint| endpoint.fresh).unwrap_or(false))
                .unwrap_or(false),
            None => false,
        },
    }
}
