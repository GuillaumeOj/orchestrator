//! Helpers shared by the integration tests.
#![allow(dead_code)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use agent_runtime::paths::AppPaths;
use agent_runtime::runtime::{Runtime, RuntimeConfig, test_plan};
use agent_runtime::server;
use orch_protocol::{AgentSpec, bus};

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

/// Per-agent pane output accumulated across reads. `Read` drains the pane, so a poll that sees
/// text while another condition still fails would otherwise lose it for every later poll.
#[derive(Default)]
pub struct Screens(std::cell::RefCell<std::collections::HashMap<String, String>>);

impl Screens {
    /// Everything the agent printed so far (this call drains what is new and appends it).
    pub fn read(&self, sock: &Path, agent_id: &str) -> String {
        let fresh = request(sock, CtlRequest::Read { agent_id: agent_id.into() }).unwrap().data;
        let mut seen = self.0.borrow_mut();
        let all = seen.entry(agent_id.to_string()).or_default();
        all.push_str(fresh.as_str().unwrap_or(""));
        all.clone()
    }
}

pub fn wait_for(what: &str, secs: u64, mut f: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("timed out waiting for {what}");
}

/// A core on a fake claude with its own config tweaks; returns what tests need.
pub struct Core {
    pub _root: tempfile::TempDir,
    pub rt: Arc<Runtime>,
    pub sock: PathBuf,
    pub repo: PathBuf,
    pub stop: Arc<AtomicBool>,
    pub srv: Option<std::thread::JoinHandle<()>>,
}

impl Core {
    pub fn start(prefix: &str, tweak: impl FnOnce(&mut RuntimeConfig)) -> Core {
        let root = short_tempdir(prefix);
        let paths = AppPaths::new(root.path().join("app"));
        agent_runtime::preflight::install_helpers(&helper_bins(), &paths.bin_dir()).unwrap();
        let fake = root.path().join("fake-claude");
        std::fs::copy(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake_claude.sh"), &fake).unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let repo = root.path().join("proj/demo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["-c", "user.name=T", "-c", "user.email=t@x", "commit", "-q", "--allow-empty", "-m", "init"]);
        let mut cfg = RuntimeConfig::new(paths.clone());
        cfg.claude = fake;
        cfg.default_plan = test_plan;
        cfg.projects_root = paths.root.join("fake-projects");
        cfg.hold = Duration::from_secs(1);
        cfg.idle_suspend = None;
        tweak(&mut cfg);
        let rt = Runtime::new(cfg).unwrap();
        let listener = bus::bind_listener(&paths.socket()).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let srv = {
            let (rt, stop) = (rt.clone(), stop.clone());
            std::thread::spawn(move || server::serve(rt, listener, stop).unwrap())
        };
        Core { _root: root, rt, sock: paths.socket(), repo, stop, srv: Some(srv) }
    }

    pub fn start_agent(&self, id: &str) {
        let spec = AgentSpec { agent_id: id.into(), repo: self.repo.display().to_string(), ..Default::default() };
        let r = request(&self.sock, CtlRequest::Start { spec: Box::new(spec) }).unwrap();
        assert!(r.ok, "{r:?}");
        request(&self.sock, CtlRequest::Deliver { agent_id: id.into(), text: "hello".into() }).unwrap();
        wait_for("first turn", 15, || agent(&self.sock, id)["turns"] == 1 && agent(&self.sock, id)["state"] == "idle");
    }

    pub fn root_path(&self) -> &Path {
        self._root.path()
    }

    pub fn state_of(&self, id: &str) -> String {
        agent(&self.sock, id)["state"].as_str().unwrap_or("").to_string()
    }
}

impl Drop for Core {
    fn drop(&mut self) {
        let _ = request(&self.sock, CtlRequest::Shutdown);
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(h) = self.srv.take() {
            let _ = h.join();
        }
    }
}
