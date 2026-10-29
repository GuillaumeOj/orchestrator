//! Orchestrator MCP tool calls (orch-mcp → bus → here). The core executes
//! them outside the agent's sandbox, after the permission rules (ADR-009):
//! a `deny` rule refuses, an `ask` rule holds the call for an Inbox
//! decision, otherwise (`allow` or no rule: the P1 default) it runs.

use anyhow::{Result, anyhow, bail};
use orch_git::commit::{CommitRequest, CommitResult};
use orch_protocol::{McpMsg, now_ms};
use orch_store::RuleEffect;
use serde_json::{Value, json};
use std::os::unix::net::UnixStream;

use crate::inbox::{Call, PARKED_MESSAGE, Waited, allows, reply};
use crate::permissions;
use crate::runtime::Runtime;
use crate::store::AgentRecord;

/// What goes back to orch-mcp: `(is_error, text for the model, data)`.
pub type McpAnswer = (bool, String, Value);

impl Runtime {
    pub fn on_mcp(&self, m: &McpMsg, conn: Option<&UnixStream>) -> McpAnswer {
        let Some(rec) = self.store.read(|s| s.agents.get(&m.agent_id).cloned()) else {
            return (true, format!("unknown agent {}", m.agent_id), Value::Null);
        };
        let tool = format!("mcp__orch__{}", m.tool);
        self.event(&rec.id, "mcp", json!({"tool": m.tool}));
        // The role's tools only (orch-mcp lists no others; checked again here).
        if !rec.role.may_call(&m.tool) {
            return (true, format!("{} is not one of your tools", m.tool), Value::Null);
        }
        // A one-shot allow (an approved parked call), then the rules
        // (deny > ask > allow), for every orchestrator tool. Each answer is
        // audited (FR-709).
        let call = Call {
            tool: tool.clone(),
            input: m.arguments.clone(),
            tool_use_id: None,
            cwd: Some(rec.worktree.display().to_string()),
        };
        let mut actor = format!("agent:{}", rec.id);
        if self.one_shot(&rec, &call) {
            actor = "user".into();
        } else {
            let rules = self.db.rules().unwrap_or_default();
            let subject = self.subject(&rec);
            if let Some(rule) = permissions::evaluate(&rules, &subject, &tool, &m.arguments) {
                let rule_actor = format!("rule:{}", rule.id);
                let detail = call.detail(None, Some(rule));
                match rule.effect {
                    RuleEffect::Deny => {
                        let _ = self.db.audit(&rule_actor, Some(&rec.id), &m.tool, Some(&tool), "deny", detail);
                        return (
                            true,
                            format!("Denied by the user's rule {} ({}). Do not retry.", rule.id, rule.pattern),
                            Value::Null,
                        );
                    }
                    RuleEffect::Ask => match self.ask_user(&rec, &call, rule.id, m.deadline_ms, conn) {
                        Ok(()) => actor = "user".into(),
                        Err(e) => return (true, format!("{e:#}"), Value::Null),
                    },
                    RuleEffect::Allow => {
                        let _ = self.db.audit(&rule_actor, Some(&rec.id), &m.tool, Some(&tool), "allow", detail);
                        actor = rule_actor;
                    }
                }
            }
        }
        if m.tool == "commit"
            && let Some(why) = self.plan_lock_reason(&rec)
        {
            return (true, why, Value::Null);
        }
        let r = match m.tool.as_str() {
            // While `rebase_base` stopped on a conflict, `commit` continues it.
            "commit" if orch_git::commit::rebasing(&orch_git::worktree::inspect(&rec.worktree)) => {
                self.rebase_continue_for(&rec)
            }
            "commit" => self.commit_args(&rec, &m.arguments, &actor).map(|c| {
                let text = format!(
                    "Committed {} on {}: {} ({} file(s): {}).",
                    &c.sha[..c.sha.len().min(12)],
                    c.branch.as_deref().unwrap_or("detached HEAD"),
                    c.summary,
                    c.files.len(),
                    c.files.iter().take(20).cloned().collect::<Vec<_>>().join(", ")
                );
                (text, json!(c))
            }),
            "request_pr" => self.pr_allowed(&rec).and_then(|()| self.request_pr(&rec, &m.arguments)).map(|t| {
                self.pr_requested(&rec.id);
                (t, Value::Null)
            }),
            // The team tools (the role's own; `may_call` checked it above).
            _ => self.team_tool(&rec, &m.tool, &m.arguments),
        };
        match r {
            Ok((text, data)) => (false, text, data),
            Err(e) => (true, format!("{e:#}"), Value::Null),
        }
    }

    /// Holds a tool call for an Inbox decision (the `ask` rule `ask_rule`),
    /// as long as a PermissionRequest is held; parked when nobody answers.
    fn ask_user(
        &self,
        rec: &AgentRecord,
        call: &Call,
        ask_rule: u64,
        deadline_ms: u64,
        conn: Option<&UnixStream>,
    ) -> Result<()> {
        let subject = self.subject(rec);
        let budget = self.hold_budget(deadline_ms);
        match self.hold_for_decision(rec, call, &subject, Some(ask_rule), budget, conn) {
            Waited::Decided(i) if allows(&i) => Ok(()),
            Waited::Decided(i) => {
                let why = reply(&i).map(|m| format!(" They said: {m}")).unwrap_or_default();
                bail!("The user declined this.{why}")
            }
            Waited::Parked => bail!("{PARKED_MESSAGE}"),
            Waited::InPane | Waited::Interrupted => bail!("Interrupted by the user."),
            Waited::Expired => {
                bail!("The user did not answer in time. Nothing was done; continue with other work or end your turn.")
            }
        }
    }

    fn commit_args(&self, rec: &AgentRecord, args: &Value, actor: &str) -> Result<CommitResult> {
        let req: CommitRequest = serde_json::from_value(args.clone()).map_err(|e| anyhow!("invalid arguments: {e}"))?;
        self.commit_for(&rec.id, &req, actor)
    }

    /// Commits in the agent's worktree on its behalf (orch-mcp `commit`, or
    /// the user from the UI with `actor = "user"`). Logged either way.
    pub fn commit_for(&self, agent_id: &str, req: &CommitRequest, actor: &str) -> Result<CommitResult> {
        let rec = self.store.read(|s| s.agents.get(agent_id).cloned()).ok_or_else(|| anyhow!("no agent {agent_id}"))?;
        let r = orch_git::commit::commit_in_worktree(&rec.worktree, req, self.agent_gitconfig(agent_id).as_deref());
        let (decision, detail) = match &r {
            Ok(c) => ("ok", json!({"sha": c.sha, "branch": c.branch, "files": c.files, "message": req.message})),
            Err(e) => ("failed", json!({"error": format!("{e:#}"), "message": req.message, "paths": req.paths})),
        };
        let _ = self.db.audit(actor, Some(agent_id), "commit", rec.worktree.to_str(), decision, detail.clone());
        self.event(agent_id, "commit", detail);
        if r.is_ok() {
            self.touch_worktree(&rec.worktree);
        }
        if let Ok(Some(p)) = self.db.project_by_path(&rec.repo) {
            let _ = self.db.touch_project(&p.id, now_ms());
        }
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::tests::test_runtime;
    use orch_git::fixtures::{git as tgit, scratch_repo};
    use orch_protocol::{InboxDecision, WorktreeKindSpec};
    use orch_store::RuleScope;
    use std::path::PathBuf;

    /// Agent `a1` on a real worktree of a scratch repo, with its gitconfig.
    fn with_worktree(rt: &Runtime) -> (tempfile::TempDir, PathBuf, PathBuf) {
        let (d, repo) = scratch_repo();
        let row = orch_git::worktree::ensure(
            &rt.store,
            &repo,
            &WorktreeKindSpec::Solo { slug: "a1".into() },
            None,
            Some("a1"),
        )
        .unwrap();
        rt.store
            .write(|s| {
                let a = s.agents.get_mut("a1").unwrap();
                a.repo = repo.clone();
                a.worktree = row.path.clone();
            })
            .unwrap();
        let dir = rt.cfg.paths.agent_dir("a1");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("gitconfig"), "[user]\n\tname = Agent\n\temail = agent@example.invalid\n").unwrap();
        orch_git::project::register(&rt.db, &repo).unwrap();
        (d, repo, row.path)
    }

    fn call(rt: &Runtime, tool: &str, args: Value) -> McpAnswer {
        rt.on_mcp(&McpMsg { agent_id: "a1".into(), tool: tool.into(), arguments: args, deadline_ms: 10_000 }, None)
    }

    #[test]
    fn commit_runs_outside_the_sandbox_logged_and_ruled() {
        let (_d, rt) = test_runtime(true);
        let (_r, _repo, wt) = with_worktree(&rt);
        std::fs::write(wt.join("a.txt"), "a").unwrap();
        let (err, text, data) = call(&rt, "commit", json!({"message": "add a", "paths": ["a.txt"]}));
        assert!(!err, "{text}");
        assert!(text.starts_with("Committed ") && text.contains("orch/solo/a1"), "{text}");
        assert_eq!(data["files"], json!(["a.txt"]));
        assert_eq!(tgit(&wt, &["log", "-1", "--format=%s"]).trim(), "add a");
        let log = rt.db.audit_log(Some("a1"), 5).unwrap();
        assert_eq!(
            (log[0].action.as_str(), log[0].decision.as_str(), log[0].actor.as_str()),
            ("commit", "ok", "agent:a1")
        );
        // Bad input is a tool error the model can read.
        let (err, text, _) = call(&rt, "commit", json!({"message": "x", "paths": ["../../etc/passwd"]}));
        assert!(err && text.contains("leaves the worktree"), "{text}");
        let (err, text, _) = call(&rt, "commit", json!({"message": "nothing"}));
        assert!(err && text.contains("nothing to commit"), "{text}");
        // A deny rule refuses before anything runs.
        let project = rt.subject(&rt.store.read(|s| s.agents["a1"].clone())).project_id.unwrap();
        let rule = rt.db.add_rule(RuleScope::Project, &project, "mcp__orch__commit", RuleEffect::Deny, None).unwrap();
        std::fs::write(wt.join("b.txt"), "b").unwrap();
        let (err, text, _) = call(&rt, "commit", json!({"message": "add b"}));
        assert!(err && text.contains("Denied by the user's rule"), "{text}");
        assert_eq!(tgit(&wt, &["log", "-1", "--format=%s"]).trim(), "add a");
        rt.db.revoke_rule(rule.id).unwrap();
        let (err, text, _) = call(&rt, "commit", json!({"message": "add b"}));
        assert!(!err, "{text}");
    }

    #[test]
    fn ask_rule_holds_the_commit_for_the_user() {
        let (_d, rt) = test_runtime(true);
        let (_r, _repo, wt) = with_worktree(&rt);
        rt.db.add_rule(RuleScope::Role, "solo", "mcp__orch__commit", RuleEffect::Ask, None).unwrap();
        std::fs::write(wt.join("c.txt"), "c").unwrap();
        let rt2 = rt.clone();
        let h = std::thread::spawn(move || call(&rt2, "commit", json!({"message": "add c"})));
        let mut item = None;
        for _ in 0..200 {
            if let Some(i) = rt.inbox().first().cloned() {
                item = Some(i);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let item = item.expect("an Inbox item");
        assert_eq!(item.kind, "permission");
        assert!(item.body.contains("mcp__orch__commit"));
        rt.decide(item.id, InboxDecision::Approve, None, None).unwrap();
        let (err, text, _) = h.join().unwrap();
        assert!(!err, "{text}");
        assert_eq!(rt.db.audit_log(Some("a1"), 1).unwrap()[0].actor, "user");
    }

    #[test]
    fn an_unanswered_ask_parks_with_the_permission_budget_and_allow_retries_once() {
        let (_d, rt) = test_runtime(true);
        let (_r, _repo, wt) = with_worktree(&rt);
        rt.db.add_rule(RuleScope::Role, "solo", "mcp__orch__commit", RuleEffect::Ask, None).unwrap();
        std::fs::write(wt.join("d.txt"), "d").unwrap();
        // 540 s of hook deadline: the hold is the 300 ms test budget, not 1 s.
        let t0 = std::time::Instant::now();
        let m = McpMsg {
            agent_id: "a1".into(),
            tool: "commit".into(),
            arguments: json!({"message": "add d"}),
            deadline_ms: 540_000,
        };
        let (err, text, _) = rt.on_mcp(&m, None);
        assert!(t0.elapsed() < std::time::Duration::from_millis(900), "{:?}", t0.elapsed());
        assert!(err && text == crate::inbox::PARKED_MESSAGE, "{text}");
        let item = rt.inbox().into_iter().next().unwrap();
        assert_eq!(item.status, orch_store::InboxStatus::Parked);
        assert_eq!(item.payload["cwd"], json!(rt.store.read(|s| s.agents["a1"].worktree.display().to_string())));
        rt.decide(item.id, InboxDecision::Approve, None, None).unwrap();
        let (err, text, _) = rt.on_mcp(&m, None);
        assert!(!err, "{text}");
        let log = rt.db.audit_log(Some("a1"), 10).unwrap();
        assert_eq!((log[0].action.as_str(), log[0].actor.as_str()), ("commit", "user"));
        assert_eq!(log[1].detail["one_shot"], true);
    }

    #[test]
    fn mcp_rule_answers_are_audited_on_every_tool() {
        let (_d, rt) = test_runtime(true);
        rt.store.write(|s| s.agents.get_mut("a1").unwrap().role.orch_tools = vec!["inbox".into()]).unwrap();
        let rule = rt.db.add_rule(RuleScope::Role, "solo", "mcp__orch__inbox", RuleEffect::Allow, None).unwrap();
        let _ = call(&rt, "inbox", json!({}));
        let log = rt.db.audit_log(Some("a1"), 5).unwrap();
        let e = log.iter().find(|e| e.action == "inbox").expect("audited");
        assert_eq!((e.actor.clone(), e.decision.as_str()), (format!("rule:{}", rule.id), "allow"));
        assert_eq!(e.detail["rule"]["pattern"], "mcp__orch__inbox");
    }

    #[test]
    fn request_pr_is_a_gate_and_a_decline_reaches_the_agent() {
        let (_d, rt) = test_runtime(true);
        let (_r, _repo, _wt) = with_worktree(&rt);
        let (err, text, _) = call(&rt, "request_pr", json!({"title": "Fix the thing", "body": "Because."}));
        assert!(!err, "{text}");
        let item = rt.inbox().into_iter().find(|i| i.kind == "gate").unwrap();
        assert_eq!(item.payload["branch"], "orch/solo/a1");
        assert_eq!(item.payload["base"], "main");
        // One request at a time.
        assert!(call(&rt, "request_pr", json!({"title": "again"})).0);
        rt.decide(item.id, InboxDecision::Reject, None, Some("not yet")).unwrap();
        assert!(rt.store.read(|s| {
            s.turns.iter().any(|t| t.agent_id == "a1" && t.body.contains("declined") && t.body.contains("not yet"))
        }));
        assert!(call(&rt, "nope", json!({})).0);
        let unknown = rt.on_mcp(
            &McpMsg { agent_id: "ghost".into(), tool: "commit".into(), arguments: json!({}), deadline_ms: 1000 },
            None,
        );
        assert!(unknown.0);
    }

    /// A runtime whose PR gate opens PRs with `open` instead of git and gh.
    fn with_opener(open: crate::inbox::PrOpener) -> (tempfile::TempDir, std::sync::Arc<Runtime>) {
        let d = tempfile::Builder::new().prefix("orchrt").tempdir_in(orch_protocol::test_tmp()).unwrap();
        let mut cfg = crate::runtime::tests::test_cfg(d.path(), true);
        cfg.open_pr = Some(open);
        let rt = Runtime::new(cfg).unwrap();
        rt.store.write(|s| s.agents.insert("a1".into(), crate::runtime::tests::test_agent(d.path()))).unwrap();
        (d, rt)
    }

    fn gate_item(rt: &Runtime) -> orch_store::InboxItem {
        rt.store.read(|s| s.inbox.iter().rev().find(|i| i.kind == "gate").cloned().unwrap())
    }

    fn wait_state(rt: &Runtime, id: u64, state: &str) -> Value {
        for _ in 0..200 {
            let lc = rt.lifecycle(id);
            if lc["state"] == state {
                return lc;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!("{id} never reached {state}: {}", rt.lifecycle(id));
    }

    #[test]
    fn the_pr_gate_has_its_diff_and_ends_pr_opened_or_failed() {
        let url = "https://github.com/o/r/pull/12";
        let ok = crate::inbox::PrOpener(std::sync::Arc::new(move |_, _, _, _, _, _| Ok(url.to_string())));
        let (_d, rt) = with_opener(ok);
        let (_r, _repo, wt) = with_worktree(&rt);
        std::fs::write(wt.join("a.txt"), "one\ntwo\n").unwrap();
        assert!(!call(&rt, "commit", json!({"message": "a"})).0);
        assert!(!call(&rt, "request_pr", json!({"title": "Add a", "body": "Because."})).0);
        let gate = gate_item(&rt);
        assert_eq!(
            gate.payload["diff"],
            json!({"files": [{"path": "a.txt", "added": 2, "removed": 0}], "added": 2, "removed": 0})
        );
        assert!(gate.payload["head"].as_str().is_some_and(|h| h.len() == 40));
        assert_eq!(
            rt.decision_patch(gate.id, Some("a.txt")).unwrap().lines().filter(|l| l.starts_with('+')).count(),
            3
        );
        let r = rt.decide(gate.id, InboxDecision::Approve, None, None).unwrap();
        assert_eq!(r["lifecycle"]["state"], "sent", "the push started");
        let lc = wait_state(&rt, gate.id, "effect");
        assert_eq!((lc["effect"].as_str(), lc["effect_detail"].as_str()), (Some("pr_opened"), Some(url)));
        // A solo PR is an outcome of the digest.
        rt.db.flush().unwrap();
        let out = rt.outcomes_since(0);
        assert!(out.iter().any(|o| o.kind == "pr_opened" && o.extra["url"] == url), "{out:?}");
    }

    #[test]
    fn a_failed_push_is_failed_and_a_moved_branch_is_stale() {
        let fail =
            crate::inbox::PrOpener(std::sync::Arc::new(|_, _, _, _, _, _| Err(anyhow::anyhow!("push rejected"))));
        let (_d, rt) = with_opener(fail);
        let (_r, _repo, wt) = with_worktree(&rt);
        std::fs::write(wt.join("a.txt"), "a").unwrap();
        call(&rt, "commit", json!({"message": "a"}));
        call(&rt, "request_pr", json!({"title": "A"}));
        let gate = gate_item(&rt);
        rt.decide(gate.id, InboxDecision::Approve, None, None).unwrap();
        let lc = wait_state(&rt, gate.id, "failed");
        assert_eq!(lc["error"], "push rejected");
        // The branch moves after the request: approving is refused, stale, and the agent asks again.
        call(&rt, "request_pr", json!({"title": "B"}));
        let gate = gate_item(&rt);
        std::fs::write(wt.join("b.txt"), "b").unwrap();
        call(&rt, "commit", json!({"message": "b"}));
        let r = rt.decide(gate.id, InboxDecision::Approve, None, None).unwrap();
        assert_eq!(r["effect"], "stale");
        assert_eq!(wait_state(&rt, gate.id, "stale")["stale_why"], "the branch moved");
        assert!(rt.store.read(|s| s.turns.iter().any(|t| t.agent_id == "a1" && t.body.contains("PR gate stale"))));
    }
}
