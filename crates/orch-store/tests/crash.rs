//! Crash safety: a writer process killed with SIGKILL mid-stream loses no
//! committed transaction and leaves a consistent database (NFR-2, NFR-8).

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

use orch_store::{AgentRow, RuntimeDiff, Store, TurnRowDb};
use serde_json::json;

const CHILD_ENV: &str = "ORCH_STORE_CRASH_CHILD";

/// Run as a child: writes runtime diffs forever, printing each committed index.
#[test]
fn child_writer() {
    let Ok(path) = std::env::var(CHILD_ENV) else { return };
    let s = Store::open(std::path::Path::new(&path)).unwrap();
    for i in 0..=u64::MAX {
        s.sync_runtime(RuntimeDiff {
            agents: vec![AgentRow {
                id: "a1".into(),
                repo: "/r".into(),
                worktree: "/w".into(),
                session_id: "s".into(),
                state: "working".into(),
                pid: None,
                tier: None,
                allocation: None,
                last_active_at_ms: Some(i),
                created_at_ms: 0,
                data: json!({"n": i}),
            }],
            turns: vec![TurnRowDb {
                id: format!("t{i}"),
                agent_id: "a1".into(),
                kind: "message".into(),
                status: "queued".into(),
                created_at_ms: i,
                delivered_at_ms: None,
                data: json!({"body": "x".repeat(200)}),
            }],
            ..Default::default()
        })
        .unwrap();
        s.add_event(i, Some("a1"), "tick", json!({"i": i}));
        println!("{i}");
    }
}

#[test]
fn sigkill_mid_write_keeps_every_committed_row() {
    let d = tempfile::tempdir().unwrap();
    let db = d.path().join("orch.db");
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "child_writer", "--nocapture", "--test-threads=1"])
        .env(CHILD_ENV, &db)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut last: Option<u64> = None;
    let out = BufReader::new(child.stdout.take().unwrap());
    for line in out.lines() {
        let Ok(line) = line else { break };
        if let Ok(n) = line.trim().parse::<u64>() {
            last = Some(n);
            if n >= 300 {
                break;
            }
        }
    }
    // SIGKILL while the writer is mid-stream.
    unsafe {
        libc_kill(child.id() as i32, 9);
    }
    let _ = child.wait();
    let last = last.expect("child committed something");

    let s = Store::open(&db).unwrap();
    let ok: String = s.read(|c| Ok(c.query_row("PRAGMA integrity_check", [], |r| r.get(0))?)).unwrap();
    assert_eq!(ok, "ok");
    let rows = s.load_runtime().unwrap();
    let n = rows.agents[0].data["n"].as_u64().unwrap();
    assert!(n >= last, "agent row {n} older than the last acknowledged commit {last}");
    // Each diff is atomic: agent n came with turn t{n}.
    assert!(rows.turns.iter().any(|t| t.id == format!("t{n}")));
    assert_eq!(rows.turns.len() as u64, n + 1);
}

unsafe extern "C" {
    #[link_name = "kill"]
    fn libc_kill(pid: i32, sig: i32) -> i32;
}
