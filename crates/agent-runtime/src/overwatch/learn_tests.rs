//! Learned rules (spec §3.7), undo of the oversight lines (§3.8), stuck
//! events (§3.6) and the leftover fixes of B2 (limit recovery, the turn with
//! no Stop).

use orch_protocol::{HookOutput, InboxDecision, now_ms};
use orch_store::{InboxItem, InboxStatus};
use serde_json::{Value, json};

use super::tests::{blocking, manual, ow_turns, t0};
use super::tools_tests::{call, deciding_item};
use super::*;
use crate::policy::Scope;
use crate::runtime::Runtime;
use crate::runtime::tests::hook;
use crate::state::AgentState;

/// A registered project that `a1` works in.
fn project(rt: &Runtime) -> (tempfile::TempDir, orch_store::Project) {
    let (d, repo) = orch_git::fixtures::scratch_repo();
    let p = orch_git::project::register(&rt.db, &repo).unwrap();
    rt.store.write(|s| s.agents.get_mut("a1").unwrap().repo = repo.clone()).unwrap();
    (d, p)
}

/// A permission of `a1` the user answers: Allow once (`allow`) or Deny.
fn answer(rt: &Runtime, p: &orch_store::Project, cmd: &str, allow: bool) -> u64 {
    let id = rt
        .add_item(
            Some("a1"),
            "permission",
            format!("a1: Bash: {cmd}"),
            json!({"tool": "Bash", "tool_input": {"command": cmd}, "cwd": "/w", "effect": "user",
                   "always": format!("Bash({cmd})"), "project_id": p.id, "role": "solo", "tool_use_id": "tu"}),
            30,
        )
        .unwrap();
    let d = if allow { InboxDecision::Approve } else { InboxDecision::Reject };
    rt.decide(id, d, None, None).unwrap();
    id
}

fn candidates(rt: &Runtime) -> Vec<Event> {
    rt.ow().queue.iter().filter(|e| e.kind == Kind::RuleCandidate).cloned().collect()
}

fn proposals(rt: &Runtime) -> Vec<InboxItem> {
    rt.store.read(|s| s.inbox.iter().filter(|i| i.kind == "rule_proposal").cloned().collect())
}

fn open_proposal(rt: &Runtime) -> InboxItem {
    proposals(rt).into_iter().find(InboxItem::open).expect("an open proposal")
}

fn behavior(o: &Option<HookOutput>) -> Option<String> {
    o.as_ref().and_then(|o| o.0.pointer("/hookSpecificOutput/decision/behavior")?.as_str().map(str::to_string))
}

fn message(o: &Option<HookOutput>) -> String {
    o.as_ref()
        .and_then(|o| o.0.pointer("/hookSpecificOutput/decision/message")?.as_str().map(str::to_string))
        .unwrap_or_default()
}

fn perm(cmd: &str) -> orch_protocol::HookMsg {
    hook(
        "PermissionRequest",
        json!({"tool_name": "Bash", "tool_input": {"command": cmd}, "tool_use_id": "tu1", "cwd": "/w"}),
    )
}

// ------------------------------------------------------------ learned rules

#[test]
fn the_third_identical_answer_is_a_candidate_for_the_overwatch() {
    let (_d, rt) = manual();
    let (_r, p) = project(&rt);
    answer(&rt, &p, "cargo fmt", true);
    answer(&rt, &p, "cargo fmt", true);
    // A different answer and a different call are other kinds of call.
    answer(&rt, &p, "cargo fmt", false);
    answer(&rt, &p, "cargo clippy", true);
    assert!(candidates(&rt).is_empty());
    let third = answer(&rt, &p, "cargo fmt", true);
    let c = candidates(&rt);
    assert_eq!(c.len(), 1, "blocking, once");
    let cand = c[0].candidate.clone().unwrap();
    assert_eq!(
        (cand["match"].as_str(), cand["effect"].as_str(), cand["scope"].as_str()),
        (Some("Bash(cargo fmt)"), Some("allow"), Some("project"))
    );
    assert_eq!((cand["project"].as_str(), cand["role"].as_str()), (Some(p.id.as_str()), Some("solo")));
    assert_eq!(cand["from"].as_array().unwrap().len(), 3);
    assert_eq!(c[0].decision, Some(format!("inbox-{third}")), "the 3rd");
    assert!(c[0].kind.blocking());
    // More answers while it is out do not make another.
    answer(&rt, &p, "cargo fmt", true);
    assert_eq!(candidates(&rt).len(), 1);
    // The threshold is a setting.
    rt.db.set_setting("permissions.hold_secs", &json!(300)).unwrap();
    let mut s = rt.overwatch_settings();
    s.rule_threshold = 2;
    rt.set_overwatch_settings(s).unwrap();
    answer(&rt, &p, "make", true);
    answer(&rt, &p, "make", true);
    assert_eq!(candidates(&rt).len(), 2);
}

#[test]
fn propose_rule_makes_a_decision_that_blocks_nobody() {
    let (_d, rt) = manual();
    let (_r, p) = project(&rt);
    let ids: Vec<String> = (0..3).map(|_| format!("inbox-{}", answer(&rt, &p, "cargo fmt", true))).collect();
    let args = json!({"match": "Bash(cargo fmt)", "effect": "allow", "scope": "project", "project": p.id,
                      "role": "solo", "from": ids, "why": "You allowed cargo fmt three times."});
    let (err, v) = call(&rt, "propose_rule", args.clone());
    assert!(!err, "{v}");
    let id = v["decision"].as_str().unwrap().to_string();
    rt.now_pump(true);
    let d = rt.open_decisions().into_iter().find(|d| d.id == id).expect("listed");
    // Level 4: not in the badge, waits on the user, never notifies; three actions.
    assert_eq!((d.kind, d.level, d.waits_on, d.counts()), ("rule_proposal", 4, "user", false));
    assert_eq!(d.actions.iter().map(|a| a.id).collect::<Vec<_>>(), ["add", "not_now", "edit"]);
    let types: Vec<&str> = d.context.iter().filter_map(|b| b["type"].as_str()).collect();
    assert_eq!(types, ["markdown", "markdown", "rule", "links"]);
    // The core's count, then what the Overwatch said; the title names the rule.
    assert!(d.context[0]["text"].as_str().unwrap().starts_with("You allowed this call 3 times"), "{}", d.context[0]);
    assert_eq!(d.context[1]["text"], "The Overwatch: You allowed cargo fmt three times.");
    assert_eq!(d.title, format!("Allow `cargo fmt` in {}?", p.id));
    let label = d.context[3]["links"][0]["label"].as_str().unwrap();
    assert!(label.ends_with(" · allowed") && label.contains(" · a1 · "), "{label}");
    let rule = &d.context[2];
    assert_eq!(
        (rule["match"].as_str(), rule["effect"].as_str(), rule["scope"].as_str()),
        (Some("Bash(cargo fmt)"), Some("allow"), Some("project"))
    );
    assert!(rule["file"].as_str().unwrap().ends_with(".orchestrator/policy.toml"), "{rule}");
    assert_eq!(d.context[3]["links"].as_array().unwrap().len(), 3);
    assert_eq!(
        (d.payload["count"].as_u64(), d.payload["threshold"].as_u64(), d.payload["declined_before"].as_u64()),
        (Some(3), Some(3), Some(0))
    );
    assert_eq!(rt.now_snapshot()["counts"]["global"]["decisions"], 0, "not counted");
    // The chat entry has its own kind.
    let e = rt.db.chat_by_decision(&id).unwrap().unwrap();
    assert_eq!(e.kind, "rule_proposal");
    // One open per kind of call.
    let (err, v) = call(&rt, "propose_rule", args);
    assert!(err && v["code"] == "duplicate", "{v}");
    // Never a rule that would allow what the core keeps.
    for pattern in ["Bash(git push*)", "Bash(git:*)", "Bash(git reset --hard*)"] {
        let (_, v) = call(
            &rt,
            "propose_rule",
            json!({"match": pattern, "effect": "allow", "scope": "global", "from": ["inbox-1"], "why": "x"}),
        );
        assert_eq!(v["code"], "not_allowed", "{pattern}: {v}");
    }
    let (_, v) = call(
        &rt,
        "propose_rule",
        json!({"match": "Bash(ls)", "effect": "allow", "scope": "project", "from": ["inbox-1"], "why": "x"}),
    );
    assert_eq!(v["code"], "bad_request", "a project scope needs a project");
}

#[test]
fn add_rule_writes_the_file_and_the_line_can_remove_it() {
    let (_d, rt) = manual();
    let (_r, p) = project(&rt);
    let path = rt.project_policy_path(&p);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "# Mine. Keep me.\nversion = 1\n").unwrap();
    let ids: Vec<String> = (0..3).map(|_| format!("inbox-{}", answer(&rt, &p, "cargo fmt", true))).collect();
    let (_, v) = call(
        &rt,
        "propose_rule",
        json!({"match": "Bash(cargo fmt)", "effect": "allow", "scope": "project",
        "project": p.id, "role": "solo", "from": ids, "why": "w"}),
    );
    let n: u64 = v["decision"].as_str().unwrap().trim_start_matches("rule-").parse().unwrap();
    let r = rt.rule_proposal_act(n, "add").unwrap();
    assert_eq!(r["effect"], "added", "{r}");
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.starts_with("# Mine. Keep me.\nversion = 1\n"), "comments kept: {text}");
    assert!(
        text.contains("match = \"Bash(cargo fmt)\"")
            && text.contains("role = \"solo\"")
            && text.contains(&format!("from = \"proposal:rule-{n}\"")),
        "{text}"
    );
    // Closed with "added to <file>"; the count starts over.
    let i = rt.store.read(|s| s.inbox.iter().find(|i| i.id == n).cloned().unwrap());
    assert_eq!((i.status, i.effect.as_deref()), (InboxStatus::Effect, Some("added")));
    assert!(
        i.effect_detail.as_deref().unwrap().starts_with("added to ")
            && i.effect_detail.as_deref().unwrap().ends_with("policy.toml"),
        "{:?}",
        i.effect_detail
    );
    assert!(rt.rule_proposal_act(n, "add").is_err(), "once");
    // The rule works: the next call is allowed by it (a1 is "solo": the rule's role).
    // The line offers "Remove rule"; the undo takes it out of the file, comments kept.
    rt.now_pump(true);
    let e = rt.db.chat_by_decision(&format!("rule-{n}")).unwrap().unwrap();
    let line = e.line.clone().unwrap();
    assert_eq!((line["verb"].as_str(), line["undo"]["label"].as_str()), (Some("added"), Some("Remove rule")));
    let r = rt.chat_undo(&e.id(), None).unwrap();
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "# Mine. Keep me.\nversion = 1\n");
    assert!(r["entry"]["line"]["undone_at_ms"].is_u64());
    // Audited as the user's (FR-709).
    let log = rt.db.audit_log(None, 10).unwrap();
    assert!(log.iter().any(|a| a.action == "chat_undo" && a.actor == "user"), "{log:?}");
    assert!(log.iter().any(|a| a.action == "rule_proposal" && a.actor == "user"));
}

#[test]
fn not_now_resets_the_count_and_two_declines_end_the_proposals() {
    let (_d, rt) = manual();
    let (_r, p) = project(&rt);
    for (declined, round) in (0..3).enumerate() {
        rt.ow().queue.clear();
        let mut last = 0;
        for _ in 0..3 {
            last = answer(&rt, &p, "cargo fmt", true);
        }
        let _ = last;
        if round == 2 {
            assert!(candidates(&rt).is_empty(), "declined twice: never proposed again");
            break;
        }
        assert_eq!(candidates(&rt).len(), 1, "round {round}");
        // The core proposes it (the Overwatch having none): then the user declines.
        let ev = candidates(&rt).remove(0);
        rt.rules_unanswered(&[ev]);
        let item = open_proposal(&rt);
        assert_eq!(item.payload["declined_before"], declined);
        let r = rt.rule_proposal_act(item.id, "not_now").unwrap();
        assert_eq!(r["effect"], "not_now");
        // "Asking again after 3 more": the count is back at zero.
        assert!(!rt.store.read(|s| s.inbox.iter().any(|i| i.kind == "rule_proposal" && i.open())));
    }
    rt.now_pump(true);
    let e = rt.db.chat_by_decision(&format!("rule-{}", proposals(&rt)[0].id)).unwrap().unwrap();
    assert_eq!(e.line.unwrap()["verb"], "not now");
}

#[test]
fn the_core_proposes_when_the_overwatch_is_down_or_makes_no_proposal() {
    // Down: at once, from the suggestion.
    let (_d, rt) = manual();
    let (_r, p) = project(&rt);
    rt.overwatch_down(why::STOPPED);
    for _ in 0..3 {
        answer(&rt, &p, "cargo fmt", true);
    }
    let item = open_proposal(&rt);
    assert_eq!(item.payload["source"], "core");
    assert!(item.payload["text"].as_str().unwrap().contains("allowed this call 3 times"), "{}", item.payload["text"]);
    // Up but silent: its turn ends without a proposal, and the core makes it.
    let (_d2, rt2) = manual();
    let (_r2, p2) = project(&rt2);
    for _ in 0..3 {
        answer(&rt2, &p2, "cargo fmt", true);
    }
    assert!(proposals(&rt2).is_empty(), "the Overwatch has it first");
    rt2.overwatch_pump_at(t0() + 1);
    assert!(ow_turns(&rt2).last().unwrap().contains("rule_candidate"));
    rt2.overwatch_turn_ended(t0() + 5_000);
    assert_eq!(open_proposal(&rt2).payload["source"], "core");
    // Up and answering: its proposal stands, the core adds none.
    let (_d3, rt3) = manual();
    let (_r3, p3) = project(&rt3);
    let ids: Vec<String> = (0..3).map(|_| format!("inbox-{}", answer(&rt3, &p3, "cargo fmt", true))).collect();
    rt3.overwatch_pump_at(t0() + 1);
    call(
        &rt3,
        "propose_rule",
        json!({"match": "Bash(cargo fmt*)", "effect": "allow", "scope": "project", "project": p3.id,
        "role": "solo", "from": ids, "why": "A prefix is enough."}),
    );
    rt3.overwatch_turn_ended(t0() + 5_000);
    assert_eq!(proposals(&rt3).len(), 1);
    assert_eq!(proposals(&rt3)[0].payload["source"], "overwatch");
}

#[test]
fn edit_rule_appends_it_commented_out_and_closes_when_the_user_turns_it_on() {
    let (_d, rt) = manual();
    let (_r, p) = project(&rt);
    rt.overwatch_down(why::STOPPED);
    for _ in 0..3 {
        answer(&rt, &p, "cargo fmt", true);
    }
    let item = open_proposal(&rt);
    let path = rt.project_policy_path(&p);
    let r = rt.rule_proposal_act(item.id, "edit").unwrap();
    assert_eq!(r["open"], "editor");
    assert_eq!(r["path"], path.display().to_string());
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        text.contains("# [[rule]]")
            && text.contains("# match = \"Bash(cargo fmt)\"")
            && text.contains("# effect = \"allow\""),
        "{text}"
    );
    // The card stays open, with the time of the edit.
    let i = rt.store.read(|s| s.inbox.iter().find(|i| i.id == item.id).cloned().unwrap());
    assert!(i.open() && i.payload["edited_at_ms"].is_u64());
    // The user removes the leading # of each line…
    let on = text
        .replace("# [[rule]]", "[[rule]]")
        .replace("# id =", "id =")
        .replace("# match =", "match =")
        .replace("# effect =", "effect =")
        .replace("# role =", "role =")
        .replace("# from =", "from =");
    std::fs::write(&path, on).unwrap();
    rt.policy_reload(&Scope::Project(p.id.clone()));
    let f = rt.policy.file(&Scope::Project(p.id.clone())).unwrap();
    assert!(f.error.is_none() && !f.doc.rules.is_empty(), "{:?} {:?}", f.error, f.doc.rules);
    let i = rt.store.read(|s| s.inbox.iter().find(|i| i.id == item.id).cloned().unwrap());
    // …and the proposal closes as added, by hand.
    assert_eq!(
        (i.status, i.effect.as_deref(), i.effect_detail.as_deref()),
        (InboxStatus::Effect, Some("added"), Some("by hand"))
    );
}

// ------------------------------------------------------------ undo

fn agent_state(rt: &Runtime, state: AgentState) {
    rt.store.write(|s| s.agents.get_mut("a1").unwrap().tracker.state = state).unwrap();
}

fn entry_of(rt: &Runtime, decision: &str) -> orch_store::ChatEntry {
    rt.now_pump(true);
    rt.db.chat_by_decision(decision).unwrap().expect("a chat entry")
}

fn turns(rt: &Runtime) -> Vec<String> {
    rt.store.read(|s| s.turns.iter().filter(|t| t.agent_id == "a1").map(|t| t.body.clone()).collect())
}

#[test]
fn undo_of_a_policy_answer_revokes_the_rule() {
    let (_d, rt) = manual();
    let (_r, p) = project(&rt);
    let r = rt
        .add_rule(orch_store::RuleScope::Project, &p.id, None, "Bash(make)", crate::policy::Effect::Allow, None)
        .unwrap();
    let rule_id = r["rule"]["id"].as_str().unwrap().to_string();
    assert_eq!(behavior(&rt.on_hook(&perm("make"), None)).as_deref(), Some("allow"));
    rt.now_pump(true);
    let e = rt
        .db
        .chat_day(&rt.overwatch_today().1)
        .unwrap()
        .into_iter()
        .find(|e| e.decision.as_deref().is_some_and(|d| d.starts_with("audit-")))
        .expect("the policy line");
    let line = e.line.clone().unwrap();
    assert_eq!(
        (line["by"].as_str(), line["undo"]["label"].as_str(), line["undo"]["until"].as_str()),
        (Some("policy"), Some("Revoke rule"), Some("the rule is in the file"))
    );
    let out = rt.chat_undo(&e.id(), None).unwrap();
    assert_eq!(out["ok"], true, "{out}");
    assert!(rt.policy.find(&rule_id, None).is_none());
    // The call is asked again (no rule), and a second undo has nothing to do.
    assert!(rt.chat_undo(&e.id(), None).unwrap_err().to_string().contains("nothing to undo"));
    // A rule removed by hand before: the undo is gone, with the reason.
    let r = rt
        .add_rule(orch_store::RuleScope::Project, &p.id, None, "Bash(make all)", crate::policy::Effect::Allow, None)
        .unwrap();
    assert_eq!(behavior(&rt.on_hook(&perm("make all"), None)).as_deref(), Some("allow"));
    rt.revoke_rule(r["rule"]["id"].as_str().unwrap(), None).unwrap();
    rt.now_pump(true);
    let mut entries = rt.db.chat_day(&rt.overwatch_today().1).unwrap();
    let e2 = entries.iter().rev().find(|e| e.line.as_ref().is_some_and(|l| l.get("undo").is_some())).unwrap().id();
    let day = rt.chat_day(&rt.overwatch_today().1).unwrap();
    let shown = day["entries"].as_array().unwrap().iter().find(|x| x["id"] == e2).unwrap().clone();
    assert!(shown["line"].get("undo").is_none(), "{shown}");
    assert_eq!(shown["line"]["undo_gone"], "the rule is not in the file any more");
    let again = rt.chat_undo(&e2, None).unwrap();
    assert_eq!(again["ok"], false);
    entries.clear();
}

#[test]
fn undoing_an_overwatch_approval_tells_the_agent_and_refuses_the_next_identical_call() {
    let (_d, rt) = manual();
    agent_state(&rt, AgentState::Idle);
    let n = deciding_item(&rt, "pnpm add zod");
    call(&rt, "decide_permission", json!({"decision": format!("inbox-{n}"), "effect": "allow", "reason": "routine"}));
    let e = entry_of(&rt, &format!("inbox-{n}"));
    let r = rt.chat_undo(&e.id(), None).unwrap();
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(
        turns(&rt).last().unwrap(),
        "ORCH: the user reversed the Overwatch's approval of `Bash: pnpm add zod`: don't repeat it; if you still need it, say why."
    );
    // The next identical call is refused, once.
    let out = rt.on_hook(&perm("pnpm add zod"), None);
    assert_eq!(behavior(&out).as_deref(), Some("deny"));
    assert!(message(&out).contains("reversed an earlier approval"), "{}", message(&out));
    assert!(rt.overwatch.denies_empty());
    assert!(r["entry"]["line"]["undone_at_ms"].is_u64());
}

#[test]
fn undoing_an_overwatch_denial_allows_that_exact_call_once() {
    let (_d, rt) = manual();
    agent_state(&rt, AgentState::Idle);
    let n = deciding_item(&rt, "pnpm add zod");
    call(&rt, "decide_permission", json!({"decision": format!("inbox-{n}"), "effect": "deny", "reason": "off task"}));
    let e = entry_of(&rt, &format!("inbox-{n}"));
    assert_eq!(e.line.as_ref().unwrap()["undo"]["label"], "Allow instead");
    let r = rt.chat_undo(&e.id(), None).unwrap();
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(
        turns(&rt).last().unwrap(),
        "ORCH: the user overrode the denial of `Bash: pnpm add zod`: you may retry it."
    );
    assert_eq!(behavior(&rt.on_hook(&perm("pnpm add zod"), None)).as_deref(), Some("allow"));
}

#[test]
fn an_undo_whose_delivery_failed_grants_one_one_shot_when_retried() {
    let (_d, rt) = manual();
    agent_state(&rt, AgentState::Idle);
    let n = deciding_item(&rt, "pnpm add zod");
    call(&rt, "decide_permission", json!({"decision": format!("inbox-{n}"), "effect": "deny", "reason": "off task"}));
    let e = entry_of(&rt, &format!("inbox-{n}"));
    rt.overwatch.fail_undo_delivery.store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(rt.chat_undo(&e.id(), None).is_err());
    assert_eq!(rt.chat_undo(&e.id(), None).unwrap()["ok"], true);
    let input = json!({"command": "pnpm add zod"});
    assert!(rt.db.take_one_shot("a1", "Bash", &input).unwrap().is_some());
    assert_eq!(rt.db.take_one_shot("a1", "Bash", &input).unwrap(), None, "exactly one");
}

#[test]
fn a_reversed_approval_whose_delivery_failed_denies_once_when_retried() {
    let (_d, rt) = manual();
    agent_state(&rt, AgentState::Idle);
    let n = deciding_item(&rt, "pnpm add zod");
    call(&rt, "decide_permission", json!({"decision": format!("inbox-{n}"), "effect": "allow", "reason": "routine"}));
    let e = entry_of(&rt, &format!("inbox-{n}"));
    rt.overwatch.fail_undo_delivery.store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(rt.chat_undo(&e.id(), None).is_err());
    assert!(rt.overwatch.denies_empty(), "rolled back");
    assert_eq!(rt.chat_undo(&e.id(), None).unwrap()["ok"], true);
    let input = json!({"command": "pnpm add zod"});
    assert!(rt.overwatch.take_deny("a1", "Bash", &input));
    assert!(rt.overwatch.denies_empty(), "exactly one");
}

#[test]
fn undoing_an_answer_needs_the_corrected_text() {
    let (_d, rt) = manual();
    agent_state(&rt, AgentState::Idle);
    let repo = rt.store.read(|s| s.agents["a1"].repo.clone());
    std::fs::create_dir_all(repo.join("docs")).unwrap();
    std::fs::write(repo.join("docs/api.md"), "# API\n").unwrap();
    let q = rt.add_item(Some("a1"), "question", "a1: Which db?".into(), json!({"question": "Which db?"}), 20).unwrap();
    let (err, v) = call(
        &rt,
        "answer_question",
        json!({"decision": format!("q-{q}"), "answers": [{"text": "sqlite"}], "sources": ["docs/api.md"]}),
    );
    assert!(!err, "{v}");
    let e = entry_of(&rt, &format!("q-{q}"));
    assert!(rt.chat_undo(&e.id(), None).unwrap_err().to_string().contains("corrected text"));
    let r = rt.chat_undo(&e.id(), Some("Postgres: see the new ADR.")).unwrap();
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(
        turns(&rt).last().unwrap(),
        "ORCH: the user corrected the Overwatch's answer to your question: Postgres: see the new ADR."
    );
    let line = &r["entry"]["line"];
    assert!(
        line["corrected_at_ms"].is_u64() && line.get("undo").is_none() && line.get("undone_at_ms").is_none(),
        "{line}"
    );
}

#[test]
fn undo_is_gone_once_the_agent_moved_on() {
    let (_d, rt) = manual();
    agent_state(&rt, AgentState::Idle);
    let n = deciding_item(&rt, "pnpm add zod");
    call(&rt, "decide_permission", json!({"decision": format!("inbox-{n}"), "effect": "allow", "reason": "routine"}));
    let e = entry_of(&rt, &format!("inbox-{n}"));
    assert!(e.line.as_ref().unwrap().get("undo").is_some());
    // Its session ended: "a1 already moved on", in the line and in the answer.
    agent_state(&rt, AgentState::Suspended);
    let r = rt.chat_undo(&e.id(), None).unwrap();
    assert_eq!((r["ok"].clone(), r["gone"].as_str()), (json!(false), Some("a1 already moved on")));
    let e = rt.db.chat_get(e.n).unwrap().unwrap();
    let line = e.line.unwrap();
    assert_eq!(line["undo_gone"], "a1 already moved on");
    assert!(line.get("undo").is_none());
    assert!(turns(&rt).is_empty(), "nothing was sent");
    // The line survives a later lifecycle move of its decision.
    rt.advance(n, crate::inbox::Stage::Effect("allowed", None));
    let e = entry_of(&rt, &format!("inbox-{n}"));
    assert_eq!(e.line.unwrap()["undo_gone"], "a1 already moved on");
}

// ------------------------------------------------------------ stuck

fn stuck_events(rt: &Runtime) -> Vec<Event> {
    rt.ow().queue.iter().filter(|e| e.kind == Kind::Stuck).cloned().collect()
}

fn in_error(rt: &Runtime) {
    rt.store
        .write(|s| {
            let a = s.agents.get_mut("a1").unwrap();
            a.tracker.state = AgentState::Error;
            a.tracker.last_error = Some("API error".into());
        })
        .unwrap();
    rt.now_pump(true);
}

#[test]
fn a_solo_session_in_error_is_stuck_at_once_and_once_per_episode() {
    let (_d, rt) = manual();
    in_error(&rt);
    let now = now_ms();
    rt.overwatch_stuck_scan(now);
    let evs = stuck_events(&rt);
    assert_eq!(evs.len(), 1, "{evs:?}");
    assert_eq!(evs[0].agent.as_deref(), Some("a1"));
    assert!(evs[0].stuck.as_ref().unwrap().why.contains("a1 is in error"), "{:?}", evs[0].stuck);
    assert!(evs[0].kind.blocking());
    rt.overwatch_stuck_scan(now + 30_000);
    assert_eq!(stuck_events(&rt).len(), 1, "once per episode");
    // It recovers, then fails again: a new episode.
    agent_state(&rt, AgentState::Idle);
    rt.now_pump(true);
    rt.overwatch_stuck_scan(now + 60_000);
    std::thread::sleep(std::time::Duration::from_millis(5));
    in_error(&rt);
    rt.overwatch_stuck_scan(now + 90_000);
    assert_eq!(stuck_events(&rt).len(), 2);
}

#[test]
fn a_team_member_reaches_the_overwatch_after_its_lead_has_had_five_minutes() {
    let (_d, rt) = manual();
    rt.store
        .write(|s| {
            let a = s.agents.get_mut("a1").unwrap();
            a.team = Some(crate::store::TeamSeat {
                run_id: "r1".into(),
                node: "back".into(),
                feature_id: "f1".into(),
                project_id: "p1".into(),
                task: Some(4),
            });
        })
        .unwrap();
    in_error(&rt);
    let now = now_ms();
    rt.overwatch_stuck_scan(now);
    assert!(stuck_events(&rt).is_empty(), "the lead hears first");
    rt.overwatch_stuck_scan(now + 4 * 60_000);
    assert!(stuck_events(&rt).is_empty());
    rt.overwatch_stuck_scan(now + 5 * 60_000 + 1);
    let evs = stuck_events(&rt);
    assert_eq!(evs.len(), 1);
    assert_eq!((evs[0].agent.as_deref(), evs[0].run.as_deref()), (Some("a1"), Some("r1")));
    // A lead is not made to wait.
    let (_d2, rt2) = manual();
    rt2.store
        .write(|s| {
            let a = s.agents.get_mut("a1").unwrap();
            a.role.orch_tools = vec!["merge".into()];
            a.team = Some(crate::store::TeamSeat {
                run_id: "r1".into(),
                node: "lead".into(),
                feature_id: "f1".into(),
                project_id: "p1".into(),
                task: None,
            });
        })
        .unwrap();
    in_error(&rt2);
    rt2.overwatch_stuck_scan(now_ms());
    assert_eq!(stuck_events(&rt2).len(), 1);
}

#[test]
fn failed_starts_and_unacted_answers_are_stuck_too() {
    let (_d, rt) = manual();
    let s = rt.add_item(Some("a1"), "startup", "a1 did not start".into(), json!({"failed": "fresh"}), 10).unwrap();
    let now = now_ms();
    rt.overwatch_stuck_scan(now);
    rt.overwatch_stuck_scan(now + 1000);
    let evs = stuck_events(&rt);
    assert_eq!(evs.len(), 1, "once");
    assert_eq!(evs[0].decision, Some(format!("startup-{s}")));
    // A parked permission the user answered, whose agent never carried on.
    let q = rt
        .add_item(
            Some("a1"),
            "permission",
            "a1: Bash: x".into(),
            json!({"tool": "Bash", "tool_input": {"command": "x"}}),
            30,
        )
        .unwrap();
    rt.store
        .write(|st| {
            let i = st.inbox.iter_mut().find(|i| i.id == q).unwrap();
            i.status = InboxStatus::Sent;
            i.sent_at_ms = Some(now - 4 * 60_000);
        })
        .unwrap();
    agent_state(&rt, AgentState::Suspended);
    rt.overwatch_stuck_scan(now);
    let evs = stuck_events(&rt);
    assert_eq!(evs.len(), 2);
    assert!(evs[1].summary.contains("was answered 4 min ago"), "{}", evs[1].summary);
}

#[test]
fn something_waiting_on_the_user_is_not_a_stuck_agent() {
    let (_d, rt) = manual();
    rt.add_item(
        Some("a1"),
        "permission",
        "a1: Bash: make".into(),
        json!({"tool": "Bash", "tool_input": {"command": "make"}, "effect": "user"}),
        30,
    )
    .unwrap();
    rt.now_pump(true);
    rt.overwatch_stuck_scan(now_ms() + 10 * 60_000);
    assert!(stuck_events(&rt).is_empty(), "the user's card; its event is decision_open");
}

// ------------------------------------------------------------ B2's leftovers

#[test]
fn a_rate_limit_lifts_when_the_window_resets() {
    let (_d, rt) = manual();
    let now = now_ms();
    rt.overwatch_rate_limited();
    let s = rt.overwatch_status();
    assert_eq!((s.status, s.why.as_deref()), (Status::Down, Some(why::LIMIT)));
    // No reading of a reset: tried again in five minutes.
    rt.overwatch_lift_limit(now + 60_000);
    assert_eq!(rt.overwatch_status().status, Status::Down);
    rt.overwatch_lift_limit(now + 5 * 60_000 + 5_000);
    assert_eq!(rt.overwatch_status().status, Status::Asleep);
    // Events flow again; a down for another reason is not lifted by it.
    assert!(rt.overwatch_enqueue(blocking("x")).is_some());
    rt.overwatch_down(why::LOGIN);
    rt.overwatch.limit_until.store(1, std::sync::atomic::Ordering::SeqCst);
    rt.overwatch_lift_limit(now + 3_600_000);
    assert_eq!(rt.overwatch_status().why.as_deref(), Some(why::LOGIN));
}

#[test]
fn a_turn_that_never_gets_a_stop_ends_past_the_ceiling() {
    let (_d, rt) = manual();
    let min = 60_000;
    rt.overwatch_enqueue(blocking("slow"));
    rt.overwatch_pump_at(t0());
    assert_eq!(rt.overwatch_status().status, Status::Working);
    // Interrupted at 10 min (team.rs); a minute of grace for the Stop.
    rt.overwatch_watchdog(t0() + 10 * min + 30_000);
    assert_eq!(rt.overwatch_status().status, Status::Working);
    rt.overwatch_watchdog(t0() + 11 * min + 1);
    let s = rt.overwatch_status();
    assert_eq!(s.status, Status::Asleep, "the turn ended: {s:?}");
    assert_eq!(rt.ow().long_turns, 1);
    assert_eq!(rt.overwatch_report()["turns"].as_array().unwrap().len(), 1);
    // A second one in a row puts it down.
    let t1 = t0() + 12 * min;
    rt.overwatch_enqueue(blocking("slow again"));
    rt.overwatch_pump_at(t1);
    rt.overwatch_watchdog(t1 + 11 * min + 1);
    let s = rt.overwatch_status();
    assert_eq!((s.status, s.why.as_deref()), (Status::Down, Some(why::TOO_SLOW)));
}

#[test]
fn the_status_is_rechecked_once_the_card_exists() {
    // The deciding mark is computed before the card is written: if the Overwatch
    // went off in between, the card is handed over right after it is posted.
    let (_d, rt) = manual();
    *rt.now.clock.lock().unwrap() = None;
    rt.db.set_setting("permissions.hold_secs", &json!(300)).unwrap();
    let n = deciding_item(&rt, "pnpm add zod");
    assert!(rt.overwatch_may_decide());
    rt.overwatch_down(why::STOPPED);
    assert!(rt.store.read(|s| s.inbox.iter().find(|i| i.id == n).unwrap().payload.get("deciding_until_ms").is_none()));
    let _ = Value::Null;
}
