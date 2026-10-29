use std::path::Path;

use serde_json::json;

use super::*;
use crate::attention::AskFacts;
use crate::store::TeamSeat;

#[derive(Default)]
struct Fx {
    items: Vec<InboxItem>,
    agents: BTreeMap<String, AgentRecord>,
    tails: BTreeMap<String, String>,
    ask: Option<AskFacts>,
    projects: BTreeMap<PathBuf, String>,
}

impl Fx {
    fn build(&self) -> Vec<Decision> {
        build(&Ctx {
            items: &self.items,
            agents: &self.agents,
            tails: &self.tails,
            ask: self.ask.as_ref(),
            runs: &[],
            projects: &self.projects,
        })
    }

    fn one(&self, id: &str) -> Decision {
        self.build().into_iter().find(|d| d.id == id).unwrap_or_else(|| panic!("no {id}"))
    }

    fn agent(&mut self, id: &str, state: AgentState) -> &mut AgentRecord {
        let mut a = crate::runtime::tests::test_agent(Path::new("/repo"));
        a.id = id.into();
        a.tracker.state = state;
        a.last_event_at_ms = Some(100);
        self.agents.insert(id.into(), a);
        self.agents.get_mut(id).unwrap()
    }

    fn member(&mut self, id: &str, node: &str) {
        self.agent(id, AgentState::Idle).team = Some(TeamSeat {
            run_id: "run-12".into(),
            node: node.into(),
            feature_id: "shop-checkout".into(),
            project_id: "shop".into(),
            task: Some(4),
        });
    }

    fn item(&mut self, agent: Option<&str>, kind: &str, body: &str, payload: Value) -> &mut InboxItem {
        let id = self.items.len() as u64 + 1;
        let mut i = InboxItem::new(id, kind, agent, body.into());
        i.payload = payload;
        i.created_at_ms = 1_000 + id;
        self.items.push(i);
        self.items.last_mut().unwrap()
    }
}

fn types(d: &Decision) -> Vec<&str> {
    d.context.iter().map(|b| b["type"].as_str().unwrap()).collect()
}

fn action_ids(d: &Decision) -> Vec<&str> {
    d.actions.iter().map(|a| a.id).collect()
}

#[test]
fn a_permission_open_then_parked() {
    let mut f = Fx::default();
    f.member("front-1", "front");
    f.item(
        Some("front-1"),
        "permission",
        "front-1: Bash: pnpm add zod",
        json!({"tool": "Bash", "tool_input": {"command": "pnpm add zod", "description": "Add zod"},
               "description": "Add zod", "cwd": "/w/front-1", "always": "Bash(pnpm add:*)"}),
    );
    let d = f.one("inbox-1");
    assert_eq!((d.kind, d.level, d.title.as_str(), d.summary.as_str()), ("permission", 1, "Bash · Add zod", "Add zod"));
    assert_eq!(types(&d), ["tool_call"]);
    assert_eq!(d.context[0]["cwd"], "/w/front-1");
    assert_eq!(action_ids(&d), ["approve", "reject", "reply", "always", "in_pane", "open"]);
    assert_eq!(d.actions[0].label, "Allow once");
    assert_eq!(
        d.subject,
        json!({"agent": "front-1", "role": "solo", "node": "front", "project": "shop", "feature": "shop-checkout",
               "run": "run-12", "task": 4})
    );
    assert_eq!((d.thread.as_deref(), d.waits_on, d.lifecycle.state), (Some("run-12"), "user", "open"));
    assert!(d.counts());
    f.items[0].status = InboxStatus::Parked;
    let d = f.one("inbox-1");
    assert_eq!((d.lifecycle.state, d.summary.as_str()), ("parked", "front-1 stopped; it resumes on your answer"));
    assert!(!action_ids(&d).contains(&"in_pane"), "no pane answer once parked");
}

#[test]
fn lifecycle_states_and_the_badge() {
    let mut f = Fx::default();
    f.agent("a", AgentState::Idle);
    let i = f.item(Some("a"), "question", "a: which db?", json!({"question": "which db?"}));
    i.status = InboxStatus::Sent;
    i.decided_by = Some("user".into());
    i.sent_at_ms = Some(5);
    let d = f.one("q-1");
    assert_eq!((d.lifecycle.state, d.waits_on, d.counts()), ("sent", "core", false));
    f.items[0].status = InboxStatus::Effect;
    f.items[0].effect = Some("resumed".into());
    let v = serde_json::to_value(&f.one("q-1").lifecycle).unwrap();
    assert_eq!(v, json!({"state": "effect", "decided_by": "user", "sent_at_ms": 5, "effect": "resumed"}));
    f.items[0].status = InboxStatus::Stale;
    f.items[0].stale_why = Some("answered in the pane".into());
    assert_eq!(f.one("q-1").lifecycle.stale_why.as_deref(), Some("answered in the pane"));
    f.items[0].status = InboxStatus::Failed;
    f.items[0].error = Some("push rejected".into());
    assert_eq!(f.one("q-1").lifecycle.state, "failed");
}

#[test]
fn a_question_with_several_questions_lists_them_all() {
    let mut f = Fx::default();
    f.member("front-1", "front");
    f.item(
        Some("front-1"),
        "question",
        "front-1: Which auth provider?",
        json!({"action": "ask_user", "question": "Which auth provider?\n\nAnd which db?", "via": "AskUserQuestion",
               "questions": [
                   {"question": "Which auth provider?", "header": "Auth", "multiSelect": false,
                    "options": [{"label": "Clerk", "description": "hosted"}, {"label": "Lucia"}]},
                   {"question": "Which db?", "header": "DB", "multiSelect": true, "options": [{"label": "pg"}, {"label": "sqlite"}]}
               ],
               "last_message": {"text": "I need these to go on.", "at_ms": 900}}),
    );
    let d = f.one("q-1");
    assert_eq!(d.title, "Which auth provider? · 2 questions");
    assert_eq!(types(&d), ["markdown", "options", "last_message"]);
    let qs = &d.context[1]["questions"];
    assert_eq!(
        qs[1],
        json!({"question": "Which db?", "header": "DB", "multi": true,
                             "options": [{"label": "pg", "description": null}, {"label": "sqlite", "description": null}]})
    );
    assert_eq!(qs[0]["options"][0]["description"], "hosted");
    assert_eq!(action_ids(&d), ["answer", "reject", "open"]);
    // `ask_user` with options: one question.
    f.item(Some("front-1"), "question", "front-1: pick", json!({"question": "pick", "options": ["a", "b"]}));
    let d = f.one("q-2");
    assert_eq!(d.context[1]["questions"][0]["options"], json!([{"label": "a"}, {"label": "b"}]));
    assert_eq!(d.thread_prev.as_deref(), Some("q-1"), "the previous decision of the run");
}

#[test]
fn gates_spec_pr_and_restart() {
    let mut f = Fx::default();
    f.member("lead-1", "lead");
    f.item(
        Some("lead-1"),
        "gate",
        "Spec gate · checkout: two tasks",
        json!({"action": "spec_approval", "feature": "checkout", "summary": "Two tasks: API and UI.",
               "docs": ["spec.md"], "doc_lines": [{"path": ".orchestrator/features/checkout/spec.md", "lines": 142}]}),
    );
    let d = f.one("gate-1");
    assert_eq!((d.kind, d.gate, d.title.as_str()), ("gate", Some("spec"), "Spec gate · checkout"));
    assert_eq!(types(&d), ["markdown", "docs"]);
    assert_eq!(d.context[1]["files"][0]["lines"], 142);
    assert_eq!(action_ids(&d), ["approve", "reject", "reply", "open"]);
    f.item(
        Some("lead-1"),
        "gate",
        "lead-1: open a PR",
        json!({"action": "push_pr", "branch": "orch/checkout/integration", "base": "main", "title": "Checkout",
               "body": "Adds checkout.", "diff": {"files": [{"path": "src/a.rs", "added": 40, "removed": 3}], "added": 40, "removed": 3}}),
    );
    let d = f.one("gate-2");
    assert_eq!((d.gate, d.title.as_str()), (Some("pr"), "PR gate · shop-checkout"));
    assert_eq!(d.summary, "orch/checkout/integration → main · 1 files changed · +40 −3 lines");
    assert_eq!(types(&d), ["markdown", "diff_summary"]);
    assert_eq!(d.context[1]["files"][0]["added"], 40);
    f.item(Some("lead-1"), "gate", "restart", json!({"action": "restart_gate", "detail": "git status not clean"}));
    let d = f.one("gate-3");
    assert_eq!((d.gate, d.title.as_str()), (Some("restart"), "Still waiting after a restart · lead-1"));
    assert_eq!(action_ids(&d), ["approve", "reject", "open"]);
}

#[test]
fn startup_alerts_budget_and_notion() {
    let mut f = Fx::default();
    f.agent("ui-1", AgentState::Starting);
    f.item(Some("ui-1"), "startup", "ui-1 did not start", json!({"tail": "a\nb\n"}));
    let d = f.one("startup-1");
    assert_eq!((d.kind, d.level, d.title.as_str()), ("startup", 2, "ui-1 did not start"));
    assert_eq!(types(&d), ["error", "pty_tail"]);
    assert_eq!(action_ids(&d), ["restart_fresh", "dismiss", "open"]);
    f.items[0].payload = json!({"failed": "fresh", "exit_status": 1});
    let d = f.one("startup-1");
    assert_eq!(
        d.context[0],
        json!({"type": "error", "text": "restarted fresh, failed again", "detail": "exit status 1"})
    );
    assert!(action_ids(&d).contains(&"delete"), "delete once a fresh start failed");

    f.agent("back-1", AgentState::Error);
    f.item(Some("back-1"), "alert", "back-1: API error 529", json!({"alert": "api_error", "error": "529"}));
    let d = f.one("alert-2");
    assert_eq!((d.kind, d.title.as_str()), ("alert", "back-1 · API error 529"));
    assert_eq!(types(&d), ["error"]);
    f.item(None, "alert", "Not on AC power\nOff-hours work won't run.", json!({"alert": "on_battery", "queued": 3}));
    let d = f.one("alert-3");
    assert_eq!(
        (d.title.as_str(), d.summary.as_str(), types(&d)),
        ("Not on AC power", "Off-hours work won't run.", vec!["markdown"])
    );
    f.item(None, "alert", "Penpot needs sign-in", json!({"alert": crate::registry::NEEDS_AUTH_ALERT}));
    assert_eq!(action_ids(&f.one("alert-4")), ["reconnect", "dismiss", "open_settings"]);
    f.item(
        None,
        "budget",
        "The reserve for your own work is used up\nAgents are squeezed harder.",
        json!({"reason": "Reserve used up"}),
    );
    let d = f.one("alert-5");
    assert_eq!((d.kind, d.level, d.title.as_str()), ("budget", 3, "Reserve used up"));
    assert_eq!(action_ids(&d), ["approve", "reject"]);
    f.item(None, "notion", "comment", json!({"source": "notion", "text": "Looks good", "author": "Ann",
                                              "page_url": "https://notion.so/p", "feature": "checkout", "doc": "spec.md"}));
    let d = f.one("alert-6");
    assert_eq!(
        (d.kind, d.title.as_str(), d.summary.as_str()),
        ("notion", "Comment on checkout · spec.md", "Ann: Looks good")
    );
    assert_eq!(d.context[0]["source"], "https://notion.so/p");
}

#[test]
fn synthesized_trust_wait_error_and_ask() {
    let mut f = Fx::default();
    f.projects.insert("/repo".into(), "blog".into());
    f.agent("t", AgentState::WaitingForYou).trust_dialog = true;
    f.tails.insert("t".into(), "Do you trust this folder?\n1. Yes".into());
    f.agent("w", AgentState::WaitingForYou);
    f.agent("held", AgentState::WaitingForYou);
    f.item(Some("held"), "permission", "held: Bash", json!({"tool": "Bash"}));
    f.agent("e", AgentState::Error).tracker.last_error = Some("model_not_found".into());
    f.ask = Some(AskFacts {
        id: 7,
        agent: Some("w".into()),
        created_ms: 50,
        cost_pp: Some(2.14),
        reason: "Queued · live cap 3/3".into(),
        effect: "back-1 runs now".into(),
        deadline_ms: 9_000,
        default_apply: false,
    });
    let all = f.build();
    let ids: Vec<&str> = all.iter().map(|d| d.id.as_str()).collect();
    assert!(ids.contains(&"trust-t") && ids.contains(&"wait-w") && ids.contains(&"error-e") && ids.contains(&"ask-7"));
    assert!(!ids.contains(&"wait-held"), "its prompt is a permission card already");
    assert!(!ids.contains(&"wait-t"), "the trust dialog wins over waiting");
    let t = f.one("trust-t");
    assert_eq!((t.kind, types(&t)), ("trust", vec!["pty_tail"]));
    assert_eq!(t.subject, json!({"agent": "t", "role": "solo", "project": "blog"}));
    let w = f.one("wait-w");
    assert_eq!((w.kind, w.title.as_str(), action_ids(&w)), ("wait", "A prompt in w's pane", vec!["reply", "open"]));
    let e = f.one("error-e");
    assert_eq!((e.kind, e.level, e.context[0]["text"].as_str()), ("error", 2, Some("model_not_found")));
    let k = f.one("ask-7");
    assert_eq!((k.kind, k.level, k.title.as_str()), ("ask", 3, "Run w now? ≈ +2.1 pp of the week"));
    assert_eq!(types(&k), ["markdown", "cost", "countdown"]);
    assert_eq!(k.context[2], json!({"type": "countdown", "default": "reject", "deadline_ms": 9_000}));
    assert_eq!(action_ids(&k), ["approve", "reject", "open"]);
}

#[test]
fn every_kind_serializes_with_subject_blocks_and_lifecycle() {
    let mut f = Fx::default();
    f.member("a", "back");
    for (kind, payload) in [
        ("permission", json!({"tool": "Bash", "tool_input": {"command": "ls"}})),
        ("question", json!({"question": "q?"})),
        ("gate", json!({"action": "spec_approval", "summary": "s"})),
        ("gate", json!({"action": "push_pr", "branch": "b", "base": "main"})),
        ("gate", json!({"action": "restart_gate", "detail": "d"})),
        ("startup", json!({})),
        ("alert", json!({"alert": "spawn_failed", "error": "e"})),
        ("budget", json!({"reason": "r"})),
        ("notion", json!({"text": "t"})),
    ] {
        f.item(Some("a"), kind, "a: x", payload);
    }
    f.agent("t", AgentState::Idle).trust_dialog = true;
    for d in f.build() {
        let v = serde_json::to_value(&d).unwrap();
        for k in [
            "id",
            "kind",
            "level",
            "subject",
            "title",
            "summary",
            "context",
            "actions",
            "lifecycle",
            "waits_on",
            "created_ms",
        ] {
            assert!(v.get(k).is_some(), "{} has no {k}: {v}", d.id);
        }
        assert!(v["subject"]["agent"].is_string(), "{v}");
        assert!(v["lifecycle"]["state"].is_string());
        assert!(!d.context.is_empty() || d.kind == "trust", "{} has blocks", d.id);
        assert!(v.get("agent").is_none(), "routing only");
    }
}
