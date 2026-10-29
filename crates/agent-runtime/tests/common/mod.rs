//! Helpers shared by the integration tests.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

use agent_runtime::server::request;
use orch_protocol::CtlRequest;
use serde_json::Value;

/// Builds the helper binaries (a no-op when up to date) and returns their folder.
/// Once per test process: parallel tests each running cargo could copy a helper
/// while another cargo replaces it (seen on a cold build).
pub fn helper_bins() -> PathBuf {
    static BUILT: std::sync::Once = std::sync::Once::new();
    BUILT.call_once(|| {
        let st = Command::new(env!("CARGO"))
            .args(["build", "-q", "-p", "orch-hook", "-p", "orch-status", "-p", "orch-mcp"])
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .status()
            .unwrap();
        assert!(st.success());
    });
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug")
}

/// git with no user/system config, panicking on failure.
pub fn git(dir: &Path, args: &[&str]) -> String {
    let o = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout).into_owned()
}

/// A short temp dir under /tmp: socket paths must stay under 104 bytes.
pub fn short_tempdir(prefix: &str) -> tempfile::TempDir {
    tempfile::Builder::new().prefix(prefix).tempdir_in(orch_protocol::test_tmp()).unwrap()
}

/// One agent's entry in the core's `state`.
pub fn agent(sock: &Path, id: &str) -> Value {
    let s = request(sock, CtlRequest::State).unwrap().data;
    s["agents"].as_array().unwrap().iter().find(|a| a["id"] == id).cloned().unwrap_or(Value::Null)
}
