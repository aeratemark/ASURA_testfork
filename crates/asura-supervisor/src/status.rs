//! The status line: what the supervisor is running, on one screen.

use crate::supervise::{Shared, State};

/// A small table: service, state, pid, restarts - and the last error for a failed one.
pub fn table(shared: &Shared) -> String {
    let sup = shared.lock().unwrap();
    let mut out = String::new();
    out.push_str(&format!(
        "supervisor: {} service(s), control on {}\n",
        sup.services.len(),
        sup.config.settings.control
    ));
    out.push_str(&format!(
        "{:<18} {:<10} {:<8} {}\n",
        "service", "state", "pid", "restarts"
    ));
    for (name, service) in &sup.services {
        out.push_str(&format!(
            "{:<18} {:<10} {:<8} {}\n",
            name,
            service.state.as_str(),
            service
                .pid
                .map(|pid| pid.to_string())
                .unwrap_or_else(|| "-".to_string()),
            service.restarts
        ));
        if service.state == State::Failed {
            if let Some(error) = &service.last_error {
                out.push_str(&format!("  {error}\n"));
            }
        }
    }
    out
}
