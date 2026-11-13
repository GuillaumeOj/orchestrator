//! P4.5 exit criterion (a), spec §1: with fake `gh`, a fake Linear
//! transport and a repo with a backlog glob, items of each source show in
//! `now.intake`; a team run and a solo session started from two of them
//! leave the lane and carry their item; a closed issue leaves the lane; a
//! brief planned by the (fake) Overwatch launches a run. The agents run on
//! the fake claude.

mod common;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use agent_runtime::intake::IntakeLink;
use agent_runtime::team::{GhFactory, LaunchSpec};
use common::{Core, agent, git, wait_for};
use orch_protocol::{AgentSpec, McpMsg};
use orch_tasks::linear::{GqlError, Graphql};
use serde_json::{Value, json};

type Answers = Arc<Mutex<BTreeMap<String, Value>>>;

/// `gh` answering by API path prefix (GET only: intake is read-only).
struct FakeGh(Answers);

impl orch_tasks::github::Gh for FakeGh {
    fn run(&self, args: &[String]) -> anyhow::Result<String> {
        // Reading writes nothing; the write-back's own writes (`-X POST …`) are accepted and ignored.
        if args.iter().any(|a| a == "-X") {
            return Ok("{}".into());
        }
        assert!(!args.iter().any(|a| a.contains("mutation")), "intake reads only: {args:?}");
        // The tree's GraphQL queries: the REST-shaped issues below, as GraphQL returns them.
        if let Some(q) = args.iter().find_map(|a| a.strip_prefix("query=")) {
            let a = self.0.lock().unwrap();
            let rest = a.get("repos/acme/shop/issues").cloned().unwrap_or(json!([]));
            if q.contains("OrchTreeMilestones") {
                return Ok(json!({"data": {"repository": {"milestones": {"nodes": []}}}}).to_string());
            }
            let nodes: Vec<Value> = rest
                .as_array()
                .into_iter()
                .flatten()
                .map(|i| {
                    let l = |k: &str, key: &str| -> Vec<Value> {
                        i[k].as_array().into_iter().flatten().map(|x| json!({key: x[key]})).collect()
                    };
                    json!({"number": i["number"], "title": i["title"], "body": i["body"], "url": i["html_url"],
                        "state": "OPEN", "updatedAt": i["updated_at"], "labels": {"nodes": l("labels", "name")},
                        "assignees": {"nodes": []}, "milestone": null, "parent": null,
                        "subIssuesSummary": {"total": 0, "completed": 0}})
                })
                .collect();
            return Ok(json!({"data": {"repository": {"url": "https://github.com/acme/shop", "issues": {
                "pageInfo": {"hasNextPage": false, "endCursor": null}, "nodes": nodes}}}})
            .to_string());
        }
        // The first argument after `api` that is not a flag (`--paginate`).
        let key = args.iter().skip(1).find(|a| !a.starts_with('-')).cloned().unwrap_or_default();
        let a = self.0.lock().unwrap();
        match a.iter().filter(|(k, _)| key.starts_with(k.as_str())).max_by_key(|(k, _)| k.len()) {
            Some((_, v)) => Ok(v.to_string()),
            None => anyhow::bail!("gh {}: HTTP 404: Not Found", args.join(" ")),
        }
    }
}

struct FakeLinear;

impl Graphql for FakeLinear {
    fn call(&self, query: &str, _vars: &Value) -> Result<Value, GqlError> {
        // The write-back's own calls are answered; reading is all else the intake does.
        if query.contains("OrchWbIssue") {
            return Ok(json!({"issue": {"id": "u1", "state": {"type": "unstarted"}, "team": {"states": {"nodes": [
                {"id": "s1", "type": "started", "position": 1.0}]}}}}));
        }
        if query.trim_start().starts_with("mutation") {
            return Ok(json!({"issueUpdate": {"success": true}, "commentCreate": {"success": true}}));
        }
        Ok(json!({"issues": {"nodes": [
            {"id": "u1", "identifier": "ENG-42", "title": "Refunds", "description": "Refund partial orders.",
             "priority": 2, "estimate": 2, "sortOrder": 1.0, "url": "https://linear.app/acme/issue/ENG-42/refunds",
             "state": {"name": "Todo", "type": "unstarted"}, "team": {"key": "ENG"}, "project": null,
             "updatedAt": "2026-11-01T00:00:00Z", "labels": {"nodes": [{"name": "next"}]}}
        ]}, "projects": {"nodes": []}}))
    }
}

fn issue(n: u64, labels: &[&str]) -> Value {
    json!({"number": n, "title": format!("Issue {n}"), "body": format!("Body of {n}"), "state": "open",
           "html_url": format!("https://github.com/acme/shop/issues/{n}"), "updated_at": "2026-11-01T00:00:00Z",
           "labels": labels.iter().map(|l| json!({"name": l})).collect::<Vec<_>>(), "assignees": []})
}

fn new_repo(root: &std::path::Path, name: &str) -> std::path::PathBuf {
    let repo = root.join("proj").join(name);
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["-c", "user.name=T", "-c", "user.email=t@x", "commit", "-q", "--allow-empty", "-m", "init"]);
    repo
}

fn items_of(rt: &agent_runtime::runtime::Runtime, project: &str) -> Vec<Value> {
    let v = rt.now_snapshot();
    v["intake"]["items"].as_array().unwrap().iter().filter(|i| i["project"] == project).cloned().collect()
}

fn refs(rt: &agent_runtime::runtime::Runtime, project: &str) -> Vec<String> {
    items_of(rt, project).iter().filter(|i| i["state"] == "ready").map(|i| i["ref"].as_str().unwrap().into()).collect()
}

#[test]
fn a_week_of_work_started_from_intake() {
    let gh: Answers = Arc::default();
    gh.lock().unwrap().insert(
        "repos/acme/shop/issues".into(),
        json!([issue(12, &["ready"]), issue(13, &["ready"]), issue(14, &["ready", "orch"])]),
    );
    let g = gh.clone();
    let core = Core::start("e2ein", move |cfg| {
        cfg.gh = Some(GhFactory(Arc::new(move || Box::new(FakeGh(g.clone())))));
        cfg.linear = Some(agent_runtime::linear::LinearFactory(Arc::new(|| Box::new(FakeLinear))));
    });
    let rt = core.rt.clone();
    // Three projects: GitHub label `ready`, Linear ENG + `next`, repo docs.
    let a = match rt.db.project_by_path(&core.repo).unwrap() {
        Some(p) => p,
        None => orch_git::project::register(&rt.db, &core.repo).unwrap(),
    };
    let b = orch_git::project::register(&rt.db, &new_repo(core.root_path(), "billing")).unwrap();
    let c_repo = new_repo(core.root_path(), "docs");
    std::fs::create_dir_all(c_repo.join(".orchestrator/backlog")).unwrap();
    orch_git::fixtures::commit_file(
        &c_repo,
        ".orchestrator/backlog/01-checkout.md",
        "---\nsize: L\n---\n# Checkout v2\n",
        "a",
    );
    let c = orch_git::project::register(&rt.db, &c_repo).unwrap();
    let (roles, team) = orch_team::presets::bugfix_squad();
    for r in &roles {
        rt.db.upsert_role(r, 1).unwrap();
    }
    rt.db.save_team(&team, &roles, 1).unwrap();
    rt.intake_source_set(
        &a.id,
        &json!({"type": "github", "repo": "acme/shop", "filter": {"kind": "label", "names": ["ready"]}}),
    )
    .unwrap();
    rt.intake_source_set(&b.id, &json!({"type": "linear", "team": "ENG", "label": "next"})).unwrap();
    rt.intake_source_set(&c.id, &json!({"type": "repo_docs"})).unwrap();

    // Items of each source show in now.intake (the orch-labelled one does not).
    assert_eq!(refs(&rt, &a.id), ["#12", "#13"]);
    assert_eq!(refs(&rt, &b.id), ["ENG-42"]);
    assert_eq!(refs(&rt, &c.id), [".orchestrator/backlog/01-checkout.md"]);
    let doc = &items_of(&rt, &c.id)[0];
    assert_eq!(doc["estimate"]["size"], "L");
    for i in items_of(&rt, &a.id) {
        assert!(i["estimate"]["team"]["high_pp"].as_f64().is_some(), "{i}");
    }

    // A team run from #12, through the prefilled launch form.
    let it12 =
        items_of(&rt, &a.id).into_iter().find(|i| i["ref"] == "#12").unwrap()["id"].as_str().unwrap().to_string();
    let form = rt.intake_prefill(Some(&it12), None, Some("team")).unwrap()["launch"].clone();
    assert!(form["brief"].as_str().unwrap().contains("#12"));
    let run = rt
        .launch(&LaunchSpec {
            project_id: a.id.clone(),
            team_id: team.id.clone(),
            slug: form["slug"].as_str().unwrap().into(),
            title: form["title"].as_str().unwrap().into(),
            brief: form["brief"].as_str().unwrap().into(),
            intake: Some(IntakeLink { item: Some(it12.clone()), ..Default::default() }),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(refs(&rt, &a.id), ["#13"], "#12 left the lane");

    // A solo session from ENG-42, its text as the first message.
    let eng = items_of(&rt, &b.id)[0]["id"].as_str().unwrap().to_string();
    let s = rt.intake_prefill(Some(&eng), None, Some("solo")).unwrap()["session"].clone();
    let name = s["name"].as_str().unwrap().to_string();
    let spec = AgentSpec {
        agent_id: name.clone(),
        repo: b.path.display().to_string(),
        first_message: s["message"].as_str().map(str::to_string),
        allocation: Some(b.id.clone()),
        ..Default::default()
    };
    rt.start_solo(&spec, &b.id, Some(&IntakeLink { item: Some(eng.clone()), ..Default::default() })).unwrap();
    assert!(refs(&rt, &b.id).is_empty(), "ENG-42 left the lane");
    wait_for("the solo session's first turn", 30, || agent(&core.sock, &name)["turns"].as_u64() >= Some(1));

    // Both rows carry their item.
    rt.now_pump(true);
    let snap = rt.now_snapshot();
    let run_row = snap["runs"].as_array().unwrap().iter().find(|r| r["run"] == run.id.as_str()).unwrap().clone();
    assert_eq!((run_row["origin"]["kind"].as_str(), run_row["origin"]["ref"].as_str()), (Some("intake"), Some("#12")));
    let solo_row = snap["agents"].as_array().unwrap().iter().find(|r| r["agent"] == name.as_str()).unwrap().clone();
    assert_eq!(solo_row["origin"]["ref"], "ENG-42");

    // #13 is closed upstream: it leaves the lane at the next refresh.
    gh.lock().unwrap().insert("repos/acme/shop/issues".into(), json!([issue(12, &["ready"])]));
    rt.intake_request(Some(&a.id)); // a user's refresh: a full read
    rt.intake_refresh_project(&a.id);
    assert!(refs(&rt, &a.id).is_empty());

    // Project C: plan with the (fake) Overwatch, which proposes a brief; the user launches it.
    let msg = rt.chat_send_with("Plan a feature for docs: ", None, Some(&json!({"plan": c.id}))).unwrap();
    let mut ow: agent_runtime::store::AgentRecord = rt.store.read(|st| st.agents[&name].clone());
    ow.id = agent_runtime::overwatch::ID.into();
    ow.overwatch = true;
    ow.role = agent_runtime::role::Role::overwatch("");
    rt.store.write(|st| st.agents.insert(ow.id.clone(), ow)).unwrap();
    let (is_err, text, out) = rt.on_mcp(
        &McpMsg {
            agent_id: agent_runtime::overwatch::ID.into(),
            tool: "propose_brief".into(),
            arguments: json!({"project": c.id, "title": "Search the docs", "brief": "Add search (docs/README.md).",
                              "mode": "team", "team": team.id, "reply_to": msg.id()}),
            deadline_ms: 5000,
        },
        None,
    );
    assert!(!is_err, "{text}");
    let decision = out["decision"].as_str().unwrap().to_string();
    let n: u64 = decision.trim_start_matches("brief-").parse().unwrap();
    assert_eq!(rt.db.runs(None, false).unwrap().len(), 1, "the proposal starts nothing");
    let opened = rt.brief_act(n, "launch").unwrap();
    let form = opened["prefill"]["launch"].clone();
    let planned = rt
        .launch(&LaunchSpec {
            project_id: c.id.clone(),
            team_id: form["team_id"].as_str().unwrap().into(),
            slug: form["slug"].as_str().unwrap().into(),
            title: form["title"].as_str().unwrap().into(),
            brief: form["brief"].as_str().unwrap().into(),
            intake: Some(IntakeLink { brief_decision: Some(decision.clone()), ..Default::default() }),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(rt.lifecycle(n)["effect"], "launched");
    rt.now_pump(true);
    let snap = rt.now_snapshot();
    let row = snap["runs"].as_array().unwrap().iter().find(|r| r["run"] == planned.id.as_str()).unwrap().clone();
    assert_eq!(row["origin"]["kind"], "brief");

    // `orchd intake --week`: three of three started from intake.
    let r = rt.intake_report(true).unwrap();
    assert_eq!(r["week"]["line"], "started from intake: 3 of 3 (1 planned with the Overwatch)");
}
