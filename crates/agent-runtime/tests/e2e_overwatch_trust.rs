//! fix-overwatch-trust-folder §6: the fake CLI shows Claude Code's trust
//! dialog (`FAKE_TRUST`, "No, exit" selected) on a real PTY. The Overwatch
//! answers its own in its day folder; a dialog that takes no key ends in the
//! fallback; a regular agent's is never answered (J0).

use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

use agent_runtime::overwatch::{Event, ID as OW, Kind, Status, why};
use orch_protocol::{AgentSpec, CtlRequest};
use serde_json::Value;

mod common;
use common::{Core, Screens, agent, wait_for};

/// A core whose `claude` is the fake with `FAKE_TRUST=<mode>` (its env is
/// cleared at spawn: a wrapper sets it).
fn core(prefix: &str, mode: &str) -> Core {
    Core::start(prefix, |c| {
        let wrapper = c.claude.with_file_name("fake-claude-trust");
        let script = format!("#!/bin/sh\nFAKE_TRUST={mode} exec '{}' \"$@\"\n", c.claude.display());
        std::fs::write(&wrapper, script).unwrap();
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
        c.claude = wrapper;
    })
}

fn events(core: &Core, kind: &str) -> Vec<Value> {
    core.rt.db.flush().unwrap();
    core.rt.db.events(0, None, Some(kind), 50).unwrap().into_iter().map(|e| e.detail).collect()
}

fn decision(core: &Core, id: &str) -> Option<Value> {
    let snap = agent_runtime::server::request(&core.sock, CtlRequest::Now).unwrap().data;
    snap["decisions"].as_array().unwrap().iter().find(|d| d["id"] == id).cloned()
}

/// Wakes the Overwatch (a user message: blocking).
fn wake(core: &Core) {
    core.rt.overwatch_enqueue(Event::new(Kind::UserMessage, "hello"));
}

fn turns(core: &Core) -> u64 {
    agent(&core.sock, OW)["turns"].as_u64().unwrap_or(0)
}

#[test]
fn the_overwatch_trusts_its_day_folder_by_itself() {
    let core = core("orchtr", "answer");
    wake(&core);
    wait_for("its first turn", 30, || turns(&core) >= 1);
    let answered = events(&core, "overwatch.trust_answered");
    assert_eq!(answered.len(), 1, "{answered:?}");
    let day = core.rt.store.read(|s| s.agents[OW].worktree.clone());
    assert_eq!(answered[0]["dir"], day.display().to_string());
    assert!(events(&core, "trust_dialog").is_empty(), "the record flag never went up");
    assert!(events(&core, "overwatch.trust_failed").is_empty());
    assert!(decision(&core, &format!("trust-{OW}")).is_none());
    let screen = Screens::default().read(&core.sock, OW);
    assert!(!screen.contains("TRUST KEY") && !screen.contains("not trusted"), "{screen}");
}

#[test]
fn a_dialog_that_takes_no_key_falls_back_to_its_session() {
    let core = core("orchts", "stuck");
    wake(&core);
    wait_for("the fallback", 30, || core.rt.overwatch_status().why.as_deref() == Some(why::TRUST));
    let s = core.rt.overwatch_status();
    assert_eq!((s.status, s.has_session), (Status::Paused, true));
    let card = decision(&core, &format!("trust-{OW}")).expect("the trust card");
    assert_eq!(card["title"], "Claude Code asks to trust today's Overwatch folder", "{card}");
    let failed = events(&core, "overwatch.trust_failed");
    assert_eq!(failed.len(), 1, "{failed:?}");
    assert!(events(&core, "overwatch.trust_answered").is_empty(), "no Enter on No");
    assert_eq!(turns(&core), 0, "nothing delivered into the dialog");
    // The user answers it in its session: SessionStart clears it all.
    let day = core.rt.store.read(|s| s.agents[OW].worktree.clone());
    std::fs::write(day.join(".fake-trust-accept"), "").unwrap();
    wait_for("its first turn", 30, || turns(&core) >= 1);
    assert_ne!(core.rt.overwatch_status().why.as_deref(), Some(why::TRUST));
    wait_for("the card gone", 5, || decision(&core, &format!("trust-{OW}")).is_none());
}

#[test]
fn a_regular_agent_s_dialog_is_never_answered() {
    let core = core("orchtj", "answer");
    let spec = AgentSpec { agent_id: "a".into(), repo: core.repo.display().to_string(), ..Default::default() };
    let r = agent_runtime::server::request(&core.sock, CtlRequest::Start { spec: Box::new(spec) }).unwrap();
    assert!(r.ok, "{r:?}");
    agent_runtime::server::request(&core.sock, CtlRequest::Deliver { agent_id: "a".into(), text: "hello".into() })
        .unwrap();
    wait_for("the card", 20, || decision(&core, "trust-a").is_some());
    assert_eq!(agent(&core.sock, "a")["trust_dialog"], true);
    // Long enough for an answer that should not come.
    std::thread::sleep(Duration::from_secs(3));
    let screen = Screens::default().read(&core.sock, "a");
    assert_eq!(screen.matches("Quick safety check").count(), 1, "never re-rendered: no arrow\n{screen}");
    assert!(!screen.contains("TRUST KEY") && !screen.contains("not trusted"), "no key\n{screen}");
    assert!(events(&core, "overwatch.trust_answered").is_empty());
    assert_eq!(agent(&core.sock, "a")["trust_dialog"], true, "still waiting for the user");
    assert_eq!(agent(&core.sock, "a")["turns"], 0, "nothing delivered into the dialog");
}
