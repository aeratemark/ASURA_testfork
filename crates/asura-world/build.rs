//! The build identity: `git describe --always --dirty` when git is available (and the repository is
//! a git checkout), the crate version otherwise. Compiled into the binary so every world publishes
//! which code it runs - the rolling deploy compares builds by this id, and "asura-world/0.1.0" was
//! the same string for every build (docs/16-implementation-plan.md, S3).

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    let described = Command::new("git")
        .args(["describe", "--always", "--dirty"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());
    println!("cargo:rustc-env=ASURA_BUILD_ID={described}");
}
