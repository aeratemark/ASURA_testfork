//! Spawning one service: its program, its arguments, and its log.
//!
//! The log is opened append-only and shared between stdout and stderr, so a service's event lines
//! (the services print theirs to stderr) and anything else it prints land in the same file, in one
//! place.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::process::Stdio;

use tokio::process::{Child, Command};

use crate::config::{repo_root, resolve_exe, ServiceConfig};

/// Start one service. Its output is appended to its log file.
pub fn spawn(name: &str, service: &ServiceConfig) -> Result<Child, String> {
    let exe = resolve_exe(&service.exe)?;
    let log_path = log_path(name, service);
    if let Some(dir) = log_path.parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir)
                .map_err(|err| format!("cannot create {}: {err}", dir.display()))?;
        }
    }
    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|err| format!("cannot open {}: {err}", log_path.display()))?;
    let _ = writeln!(log, "== {name}: started ==");
    let stdout = log
        .try_clone()
        .map_err(|err| format!("cannot clone the log handle: {err}"))?;
    let mut command = Command::new(exe);
    command
        .args(&service.args)
        .current_dir(repo_root())
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(log));
    // A Ctrl+C in the supervisor's console must stop the services *through the supervisor*, not
    // kill them behind its back. The children are put in their own process group, where the
    // console's Ctrl+C does not reach them: Windows disables Ctrl+C for a
    // CREATE_NEW_PROCESS_GROUP group, and a Unix child in its own process group does not get the
    // terminal's SIGINT either.
    #[cfg(windows)]
    {
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        command.creation_flags(CREATE_NEW_PROCESS_GROUP);
    }
    #[cfg(unix)]
    {
        command.process_group(0);
    }
    command
        .spawn()
        .map_err(|err| format!("{name}: cannot start {:?}: {err}", service.exe))
}

/// The log file a service appends to: the configured one, or `logs/<name>.log`.
pub fn log_path(name: &str, service: &ServiceConfig) -> PathBuf {
    let configured = service
        .log
        .clone()
        .unwrap_or_else(|| format!("logs/{name}.log"));
    let path = PathBuf::from(configured);
    if path.is_absolute() {
        path
    } else {
        repo_root().join(path)
    }
}
