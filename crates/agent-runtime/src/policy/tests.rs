//! Policy: evaluation (merge, precedence, scopes, defaults, hard lists),
//! the files in the Runtime (load, parse errors, the watcher, writes),
//! migration 14 and the permission paths.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use orch_protocol::{HookOutput, InboxDecision, now_ms};
use orch_store::{InboxStatus, RuleEffect, RuleScope};
use serde_json::{Value, json};

use super::hard::{self, HardKind, HardVerdict};
use super::*;
use crate::permissions::Subject;
use crate::runtime::Runtime;
use crate::runtime::tests::{hook, test_cfg, test_runtime};

fn file(scope: Scope, text: &str) -> FileState {
    let mut f = FileState::new(scope, PathBuf::from("/x/policy.toml"));
    f.doc = file::parse(text).unwrap();
    f.exists = true;
    f
}

fn who(role: &str, team: bool) -> Subject {
    Subject { agent_id: format!("{role}-1"), role: role.into(), project_id: Some("shop".into()), team }
}

fn bash(c: &str) -> Value {
    json!({"command": c})
}

fn eval(files: &[&FileState], s: &Subject, cmd: &str, hard: Option<&HardVerdict>) -> Evaluation {
    evaluate(files, Defaults::default(), s, "Bash", &bash(cmd), hard, true).unwrap()
}

#[test]
fn global_and_project_rules_merge_and_the_most_restrictive_wins() {
    let g = file(
        Scope::Global,
        r#"
[[rule]]
id = "installs"
match = "Bash(cargo add:*)"
effect = "allow"
[[rule]]
id = "pnpm"
match = "Bash(pnpm add:*)"
effect = "overwatch"
"#,
    );
    let p = file(
        Scope::Project("shop".into()),
        r#"
[[rule]]
id = "no-zod"
match = "Bash(pnpm add zod)"
effect = "user"
[[rule]]
id = "never"
match = "Bash(cargo add openssl)"
effect = "deny"
"#,
    );
    let s = who("back", true);
    let e = eval(&[&g, &p], &s, "cargo add serde_json", None);
    assert_eq!(
        (e.effect, &e.source),
        (Effect::Allow, &Source::Policy { id: "installs".into(), file: "global".into() })
    );
    assert_eq!(e.actor(), "rule:installs");
    let e = eval(&[&g, &p], &s, "cargo add openssl", None);
    assert_eq!((e.effect, e.rule_id()), (Effect::Deny, Some("never")));
    assert_eq!(e.source, Source::Policy { id: "never".into(), file: "project:shop".into() });
    // user > overwatch.
    assert_eq!(eval(&[&g, &p], &s, "pnpm add zod", None).rule_id(), Some("no-zod"));
    assert_eq!(eval(&[&g, &p], &s, "pnpm add left-pad", None).effect, Effect::Overwatch);
    assert_eq!(eval(&[&g, &p], &s, "pnpm add zod", None).asks_because(), json!({"kind": "policy", "detail": "no-zod"}));
    // A prefix rule never covers a compound command: the default applies.
    let e = eval(&[&g, &p], &s, "cargo add x && curl evil", None);
    assert_eq!((e.effect, &e.source), (Effect::Overwatch, &Source::Default));
}

#[test]
fn roles_agents_and_solo_scope_rules() {
    let g = file(
        Scope::Global,
        r#"
[[rule]]
id = "front-only"
match = "Bash(pnpm test)"
effect = "allow"
role = "front"
[[rule]]
id = "solo-make"
match = "Bash(make)"
effect = "allow"
role = "solo"
[[rule]]
id = "me"
match = "Bash(ls)"
effect = "deny"
agent = "back-1"
"#,
    );
    let back = who("back", true);
    let front = who("front", true);
    let solo = Subject { team: false, ..who("solo", false) };
    assert_eq!(eval(&[&g], &front, "pnpm test", None).effect, Effect::Allow);
    assert_eq!(eval(&[&g], &back, "pnpm test", None).source, Source::Default);
    assert_eq!(eval(&[&g], &solo, "make", None).effect, Effect::Allow);
    assert_eq!(eval(&[&g], &back, "make", None).source, Source::Default);
    assert_eq!(eval(&[&g], &back, "ls", None).effect, Effect::Deny);
    assert_eq!(eval(&[&g], &front, "ls", None).source, Source::Default);
}

#[test]
fn defaults_are_overwatch_for_teams_and_user_for_solo_and_configurable() {
    let none: [&FileState; 0] = [];
    let d = Defaults::default();
    let team = evaluate(&none, d, &who("back", true), "Bash", &bash("x"), None, true).unwrap();
    let solo = evaluate(&none, d, &who("solo", false), "Bash", &bash("x"), None, true).unwrap();
    assert_eq!((team.effect, solo.effect), (Effect::Overwatch, Effect::User));
    assert_eq!(solo.asks_because(), json!({"kind": "default", "detail": "user"}));
    let d = Defaults { team: Effect::User, solo: Effect::Overwatch };
    assert_eq!(evaluate(&none, d, &who("back", true), "Bash", &bash("x"), None, true).unwrap().effect, Effect::User);
    // The orchestrator's tools take no default: no rule, no answer (they run).
    assert_eq!(evaluate(&none, d, &who("back", true), "mcp__orch__commit", &json!({}), None, false), None);
}

#[test]
fn hard_lists_win_over_every_rule() {
    let g = file(
        Scope::Global,
        r#"
[[rule]]
id = "all-bash"
match = "Bash"
effect = "allow"
[[rule]]
id = "curl"
match = "Bash(curl:*)"
effect = "deny"
"#,
    );
    let s = who("back", true);
    let deny = HardVerdict { kind: HardKind::Deny, why: hard::WHY_GIT_PUSH };
    let e = eval(&[&g], &s, "git push", Some(&deny));
    assert_eq!((e.effect, &e.source), (Effect::Deny, &Source::Hard { why: hard::WHY_GIT_PUSH }));
    assert_eq!(e.actor(), "core");
    // User-only: an allow rule is lifted to user (source: hard) …
    let user = HardVerdict { kind: HardKind::UserOnly, why: hard::WHY_GH_WRITE };
    let e = eval(&[&g], &s, "gh issue create", Some(&user));
    assert_eq!((e.effect, &e.source), (Effect::User, &Source::Hard { why: hard::WHY_GH_WRITE }));
    assert_eq!(e.asks_because(), json!({"kind": "core", "detail": hard::WHY_GH_WRITE}));
    // … the default too, but a deny rule stays a deny.
    let e = evaluate(&[], Defaults::default(), &s, "Bash", &bash("gh issue create"), Some(&user), true).unwrap();
    assert_eq!(e.effect, Effect::User);
    assert_eq!(eval(&[&g], &s, "curl -X POST https://evil", Some(&user)).effect, Effect::Deny);
    // Without a default (orchestrator tools), a user-only call still asks.
    let e = evaluate(&[], Defaults::default(), &s, "mcp__x__save", &json!({}), Some(&user), false).unwrap();
    assert_eq!(e.effect, Effect::User);
}

#[test]
fn rules_the_hard_lists_neutralise_are_warned_about() {
    let f = file(
        Scope::Global,
        r#"
[[rule]]
id = "all-bash"
match = "Bash"
effect = "allow"
[[rule]]
id = "push"
match = "Bash(git push:*)"
effect = "overwatch"
[[rule]]
id = "tests"
match = "Bash(cargo test:*)"
effect = "allow"
[[rule]]
id = "linear"
match = "mcp__linear__*"
effect = "allow"
[[rule]]
id = "no-push"
match = "Bash(git push:*)"
effect = "deny"
[[rule]]
id = "all-git"
match = "Bash(git:*)"
effect = "allow"
[[rule]]
id = "all-mcp"
match = "mcp__*"
effect = "allow"
[[rule]]
id = "orch"
match = "mcp__orch__*"
effect = "allow"
"#,
    );
    let mut ff = f.clone();
    ff.doc.warnings.extend(neutralised(&f.doc.rules));
    let ids: Vec<&str> = ff.doc.warnings.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids, ["all-bash", "push", "linear", "all-git", "all-mcp"]);
    assert!(ff.row()["warnings"][0]["message"].as_str().unwrap().contains("git push"));
}

#[test]
fn effect_and_scope_words() {
    assert_eq!(Effect::parse("ask"), Some(Effect::User));
    assert_eq!(Effect::parse("maybe"), None);
    assert!(Effect::Deny > Effect::User && Effect::User > Effect::Overwatch && Effect::Overwatch > Effect::Allow);
    assert_eq!(Scope::parse("project:shop"), Some(Scope::Project("shop".into())));
    assert_eq!(Scope::parse("global"), Some(Scope::Global));
    assert_eq!(Scope::parse("project:"), None);
    assert_eq!(serde_json::to_value(Source::Hard { why: "x" }).unwrap(), json!({"kind": "hard", "why": "x"}));
}

// ------------------------------------------------------------------ Runtime

fn wait(what: &str, secs: u64, mut ok: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        if ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("timed out: {what}");
}

fn perm(cmd: &str) -> orch_protocol::HookMsg {
    hook("PermissionRequest", json!({"tool_name": "Bash", "tool_input": {"command": cmd}, "tool_use_id": "tu1"}))
}

fn behavior(o: &Option<HookOutput>) -> Option<String> {
    o.as_ref().and_then(|o| o.0.pointer("/hookSpecificOutput/decision/behavior")?.as_str().map(str::to_string))
}

fn message(o: &Option<HookOutput>) -> String {
    let o = o.as_ref().unwrap();
    o.0.pointer("/hookSpecificOutput/decision/message").and_then(Value::as_str).unwrap_or("").to_string()
}

fn policy_alerts(rt: &Runtime) -> Vec<orch_store::InboxItem> {
    rt.inbox().into_iter().filter(|i| i.kind == "alert" && i.payload["alert"] == POLICY_ALERT).collect()
}

#[test]
fn the_global_file_is_read_and_a_parse_error_keeps_the_last_good_version() {
    let (_d, rt) = test_runtime(true);
    let path = rt.global_policy_path();
    std::fs::write(&path, "version = 1\n[[rule]]\nid = \"t\"\nmatch = \"Bash(make)\"\neffect = \"allow\"\n").unwrap();
    rt.policy_reload(&Scope::Global);
    assert_eq!(behavior(&rt.on_hook(&perm("make"), None)).as_deref(), Some("allow"));
    assert_eq!(rt.db.audit_log(Some("a1"), 1).unwrap()[0].actor, "rule:t");
    // Broken: the rule stays in effect and one alert names the file and line.
    std::fs::write(&path, "version = 1\n[[rule]]\nid = \"t\"\nmatch = = \"Bash(make)\"\n").unwrap();
    rt.policy_reload(&Scope::Global);
    rt.policy_reload(&Scope::Global);
    assert_eq!(behavior(&rt.on_hook(&perm("make"), None)).as_deref(), Some("allow"), "last good version");
    let alerts = policy_alerts(&rt);
    assert_eq!(alerts.len(), 1, "one alert per file");
    assert_eq!((alerts[0].payload["line"].as_u64(), alerts[0].payload["file"].as_str()), (Some(4), path.to_str()));
    assert!(alerts[0].body.contains("line 4"), "{}", alerts[0].body);
    let row = rt.policy_files().into_iter().find(|f| f["scope"] == "global").unwrap();
    assert_eq!(row["error"]["line"], 4);
    // Writes refuse to touch a broken file.
    assert!(rt.add_rule(RuleScope::Role, "solo", None, "Bash(x)", Effect::Allow, None).is_err());
    // Fixed: the alert closes as stale, the new rules apply.
    std::fs::write(&path, "version = 1\n").unwrap();
    rt.policy_reload(&Scope::Global);
    assert!(policy_alerts(&rt).is_empty());
    assert_eq!(rt.db.audit_log(Some("a1"), 1).unwrap()[0].actor, "rule:t");
    let rt2 = rt.clone();
    let h = std::thread::spawn(move || rt2.on_hook(&perm("make"), None));
    wait("held for the user", 5, || rt.inbox().iter().any(|i| i.kind == "permission"));
    let item = rt.inbox().into_iter().find(|i| i.kind == "permission").unwrap();
    assert_eq!(item.payload["asks_because"], json!({"kind": "default", "detail": "user"}));
    assert_eq!(item.payload["effect"], "user");
    let _ = h.join();
}

#[test]
fn the_watcher_reloads_an_edited_file_after_its_debounce() {
    let (_d, rt) = test_runtime(true);
    let path = rt.global_policy_path();
    std::fs::write(&path, "version = 1\n[[rule]]\nid = \"w\"\nmatch = \"Bash(make)\"\neffect = \"deny\"\n").unwrap();
    wait("the watcher reads the new rule", 15, || rt.policy_rules(None).iter().any(|r| r["id"] == "w"));
    assert_eq!(behavior(&rt.on_hook(&perm("make"), None)).as_deref(), Some("deny"));
    std::fs::write(&path, "[[rule]\n").unwrap();
    wait("the watcher raises the alert", 15, || !policy_alerts(&rt).is_empty());
    assert_eq!(rt.policy_rules(None).len(), 1, "last good kept");
}

#[test]
fn a_watched_path_reloads_its_file() {
    let (_d, rt) = test_runtime(true);
    let path = rt.global_policy_path();
    std::fs::write(&path, "[[rule]]\nid = \"p\"\nmatch = \"Bash\"\neffect = \"deny\"\n").unwrap();
    // FSEvents reports the real path (`/private/tmp/…` for `/tmp/…`).
    let real = std::fs::canonicalize(&path).unwrap();
    rt.policy_reload_path(&real);
    assert_eq!(rt.policy_rules(None)[0]["id"], "p");
    rt.policy_reload_path(Path::new("/elsewhere/policy.toml"));
    assert_eq!(rt.policy_rules(None).len(), 1);
}

#[test]
fn hard_denies_answer_at_once_and_are_the_cores() {
    let (_d, rt) = test_runtime(true);
    rt.add_rule(RuleScope::Role, "solo", None, "Bash", Effect::Allow, None).unwrap();
    let o = rt.on_hook(&perm("cargo test && git push origin HEAD"), None);
    assert_eq!(behavior(&o).as_deref(), Some("deny"));
    assert!(message(&o).contains(hard::WHY_GIT_PUSH), "{}", message(&o));
    assert!(rt.inbox().is_empty());
    let log = rt.db.audit_log(Some("a1"), 1).unwrap();
    assert_eq!((log[0].actor.as_str(), log[0].decision.as_str()), ("core", "deny"));
    assert_eq!(log[0].detail["hard"], hard::WHY_GIT_PUSH);
    // Even a one-shot allow does not let it through.
    let input = json!({"command": "git push"});
    rt.db.add_one_shot("a1", "Bash", &input, None, now_ms() + 60_000).unwrap();
    assert_eq!(behavior(&rt.on_hook(&perm("git push"), None)).as_deref(), Some("deny"));
    // A user-only call waits for the user despite the allow rule, and can't become a rule.
    let rt2 = rt.clone();
    let h = std::thread::spawn(move || rt2.on_hook(&perm("gh issue create -t x"), None));
    wait("held", 5, || rt.inbox().iter().any(|i| i.kind == "permission"));
    let item = rt.inbox().into_iter().find(|i| i.kind == "permission").unwrap();
    assert_eq!(item.payload["asks_because"], json!({"kind": "core", "detail": hard::WHY_GH_WRITE}));
    assert_eq!(item.payload["always"], Value::Null);
    let e = rt.decide(item.id, InboxDecision::AlwaysAllow, None, None).unwrap_err();
    assert!(e.to_string().contains("keeps this call for you"), "{e}");
    rt.decide(item.id, InboxDecision::Approve, None, None).unwrap();
    assert_eq!(behavior(&h.join().unwrap()).as_deref(), Some("allow"));
}

#[test]
fn a_user_only_call_takes_a_one_shot_only_from_the_users_decision() {
    let (_d, rt) = test_runtime(true);
    let input = json!({"command": "git reset --hard"});
    // A one-shot nobody decided (no item): not enough for a user-only call.
    rt.db.add_one_shot("a1", "Bash", &input, None, now_ms() + 60_000).unwrap();
    let rec = rt.store.read(|s| s.agents["a1"].clone());
    let call = crate::inbox::Call { tool: "Bash".into(), input: input.clone(), tool_use_id: None, cwd: None };
    let hard = rt.hard_check(&rec, "Bash", &input, None);
    assert_eq!(hard.as_ref().map(|h| h.kind), Some(HardKind::UserOnly));
    assert!(!rt.one_shot(&rec, &call, hard.as_ref()));
    // The parked-retry path: the user approved this exact call.
    let id = rt.store.write(|s| s.add_inbox(Some("a1"), "permission", "x".into())).unwrap();
    rt.store
        .write(|s| s.close_inbox(id, InboxStatus::Decided, Some("approve".into()), "user").map(|_| ()))
        .unwrap()
        .unwrap();
    rt.db.add_one_shot("a1", "Bash", &input, Some(id), now_ms() + 60_000).unwrap();
    assert!(rt.one_shot(&rec, &call, hard.as_ref()));
}

fn project(rt: &Runtime) -> (tempfile::TempDir, orch_store::Project) {
    let (d, repo) = orch_git::fixtures::scratch_repo();
    let p = orch_git::project::register(&rt.db, &repo).unwrap();
    rt.store.write(|s| s.agents.get_mut("a1").unwrap().repo = repo.clone()).unwrap();
    (d, p)
}

#[test]
fn project_files_live_in_the_project_or_the_app_data() {
    let (_d, rt) = test_runtime(true);
    let (_r, p) = project(&rt);
    assert_eq!(rt.project_policy_path(&p), p.path.join(".orchestrator/policy.toml"));
    let untracked = orch_store::Project { track_orchestrator_dir: false, ..p.clone() };
    assert_eq!(rt.project_policy_path(&untracked), rt.cfg.paths.root.join("projects").join(&p.id).join("policy.toml"));
    // policy_files lists every file, missing ones too (F15).
    let files = rt.policy_files();
    assert_eq!(files.len(), 2);
    assert_eq!((files[0]["scope"].as_str(), files[0]["exists"].as_bool()), (Some("global"), Some(false)));
    assert_eq!((files[1]["project"].as_str(), files[1]["rules"].as_u64()), (Some(p.id.as_str()), Some(0)));
    // policy_open creates it with its header.
    let path = rt.policy_open("project", Some(&p.id)).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("version = 1") && text.starts_with('#'), "{text}");
    assert!(rt.policy_files()[1]["exists"].as_bool().unwrap());
    assert!(rt.policy_open("project", None).is_err());
    // A project rule applies to that project's agents only (a1 is in it now).
    rt.add_rule(RuleScope::Project, &p.id, None, "Bash(make)", Effect::Deny, None).unwrap();
    assert_eq!(behavior(&rt.on_hook(&perm("make"), None)).as_deref(), Some("deny"));
    let log = rt.db.audit_log(Some("a1"), 1).unwrap();
    assert_eq!(log[0].detail["rule"]["file"], format!("project:{}", p.id));
    let after = std::fs::read_to_string(&path).unwrap();
    assert!(after.starts_with(&text) && after.contains("Bash(make)"), "{after}");
}

#[test]
fn always_allow_writes_the_projects_file_and_undo_removes_it() {
    let (_d, rt) = test_runtime(true);
    let (_r, p) = project(&rt);
    let path = rt.project_policy_path(&p);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "# Mine. Keep me.\nversion = 1\n").unwrap();
    let rt2 = rt.clone();
    let h = std::thread::spawn(move || rt2.on_hook(&perm("cargo fmt"), None));
    wait("held", 5, || rt.inbox().iter().any(|i| i.kind == "permission"));
    let item = rt.inbox().into_iter().find(|i| i.kind == "permission").unwrap();
    let r = rt.decide(item.id, InboxDecision::AlwaysAllow, Some("project"), None).unwrap();
    let _ = h.join();
    assert_eq!(r["rule"]["file"], format!("project:{}", p.id));
    let id = r["rule"]["rule"]["id"].as_str().unwrap().to_string();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.starts_with("# Mine. Keep me.\nversion = 1\n"), "{text}");
    assert!(text.contains("match = \"Bash(cargo fmt)\"") && text.contains(&format!("from = \"inbox-{}\"", item.id)));
    assert_eq!(behavior(&rt.on_hook(&perm("cargo fmt"), None)).as_deref(), Some("allow"));
    assert_eq!(rt.db.audit_log(Some("a1"), 1).unwrap()[0].actor, format!("rule:{id}"));
    assert!(rt.revoke_rule(&id, Some(&format!("project:{}", p.id))).unwrap());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "# Mine. Keep me.\nversion = 1\n");
    let log = rt.db.audit_log(None, 5).unwrap();
    assert!(log.iter().any(|e| e.action == "rule_revoked" && e.actor == "user" && e.decision == "ok"));
}

#[test]
fn defaults_are_set_in_the_global_file() {
    let (_d, rt) = test_runtime(true);
    assert_eq!(rt.policy_defaults(), Defaults::default());
    let d = rt.set_policy_defaults("user", "overwatch").unwrap();
    assert_eq!(d, Defaults { team: Effect::User, solo: Effect::Overwatch });
    assert!(std::fs::read_to_string(rt.global_policy_path()).unwrap().contains("[defaults]"));
    assert!(rt.set_policy_defaults("allow", "user").is_err());
    let t = rt.policy_test(None, "solo", "Bash", &json!({"command": "make"})).unwrap();
    assert_eq!(t["evaluation"]["effect"], "overwatch");
    assert_eq!(t["evaluation"]["source"]["kind"], "default");
    let t = rt.policy_test(None, "back", "Bash", &json!({"command": "git push"})).unwrap();
    assert_eq!((t["evaluation"]["effect"].as_str(), t["hard"]["kind"].as_str()), (Some("deny"), Some("deny")));
}

#[test]
fn the_orchestrators_tools_run_with_no_rule_and_hard_checks_apply() {
    let (_d, rt) = test_runtime(true);
    rt.store.write(|s| s.agents.get_mut("a1").unwrap().role.orch_tools = vec!["inbox".into()]).unwrap();
    let m = |args: Value| orch_protocol::McpMsg {
        agent_id: "a1".into(),
        tool: "inbox".into(),
        arguments: args,
        deadline_ms: 10_000,
    };
    // No rule: it runs (the solo default `user` is not used for orchestrator tools).
    let (_err, _text, _) = rt.on_mcp(&m(json!({})), None);
    assert!(rt.inbox().iter().all(|i| i.kind != "permission"));
    // A path argument into a credential store is the core's deny.
    let (err, text, _) = rt.on_mcp(&m(json!({"path": "~/.ssh/id_rsa"})), None);
    assert!(err && text.contains(hard::WHY_CREDENTIALS), "{text}");
    assert_eq!(rt.db.audit_log(Some("a1"), 1).unwrap()[0].actor, "core");
}

// ------------------------------------------------------------------ migration 14

fn seed_rules(dir: &Path) -> (orch_store::Project, Vec<u64>) {
    let db = orch_store::Store::open(&dir.join("app/state/orch.db")).unwrap();
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    orch_git::fixtures::git(&repo, &["init", "-q", "-b", "main"]);
    let p = orch_git::project::register(&db, &repo).unwrap();
    let ids = vec![
        db.add_rule(RuleScope::Project, &p.id, "Bash(cargo add:*)", RuleEffect::Allow, Some(3)).unwrap().id,
        db.add_rule(RuleScope::Role, "back", "Bash(rm:*)", RuleEffect::Ask, None).unwrap().id,
        db.add_rule(RuleScope::Agent, "ghost", "Edit", RuleEffect::Deny, None).unwrap().id,
        db.add_rule(RuleScope::Project, &p.id, "Bash(ls)", RuleEffect::Allow, None).unwrap().id,
        db.add_rule(RuleScope::Project, "gone", "Bash", RuleEffect::Allow, None).unwrap().id,
    ];
    db.revoke_rule(ids[3]).unwrap();
    (p, ids)
}

#[test]
fn migration_14_exports_store_rules_once() {
    let d = tempfile::Builder::new().prefix("orchmig").tempdir_in(orch_protocol::test_tmp()).unwrap();
    let (p, ids) = seed_rules(d.path());
    let rt = Runtime::new(test_cfg(d.path(), true)).unwrap();
    let rules = rt.policy_rules(None);
    let by_id = |id: u64| rules.iter().find(|r| r["id"] == format!("r{id}")).cloned();
    let proj = by_id(ids[0]).unwrap();
    assert_eq!(
        (proj["match"].as_str(), proj["effect"].as_str(), proj["file"].as_str()),
        (Some("Bash(cargo add:*)"), Some("allow"), Some(format!("project:{}", p.id).as_str()))
    );
    assert_eq!(proj["from"], "store");
    let role = by_id(ids[1]).unwrap();
    assert_eq!(
        (role["effect"].as_str(), role["role"].as_str(), role["file"].as_str()),
        (Some("user"), Some("back"), Some("global"))
    );
    let agent = by_id(ids[2]).unwrap();
    assert_eq!((agent["agent"].as_str(), agent["file"].as_str()), (Some("ghost"), Some("global")));
    assert!(by_id(ids[3]).is_none(), "revoked rows are not exported");
    assert!(by_id(ids[4]).is_none(), "a rule of a removed project is dropped");
    assert_eq!(rules.len(), 3);
    assert!(rt.db.rules_to_migrate().unwrap().is_empty(), "every row marked");
    assert!(std::fs::read_to_string(p.path.join(".orchestrator/policy.toml")).unwrap().contains("Bash(cargo add:*)"));
    drop(rt);
    // A second start exports nothing more.
    let rt = Runtime::new(test_cfg(d.path(), true)).unwrap();
    assert_eq!(rt.policy_rules(None).len(), 3);
    let g = std::fs::read_to_string(rt.global_policy_path()).unwrap();
    assert_eq!(g.matches("\n[[rule]]").count(), 2);
}

#[test]
fn the_store_table_is_no_longer_read() {
    let (_d, rt) = test_runtime(true);
    rt.db.add_rule(RuleScope::Role, "solo", "Bash(make)", RuleEffect::Allow, None).unwrap();
    let rt2: Arc<Runtime> = rt.clone();
    let h = std::thread::spawn(move || rt2.on_hook(&perm("make"), None));
    wait("held: the store row is ignored", 5, || rt.inbox().iter().any(|i| i.kind == "permission"));
    let _ = h.join();
}

#[test]
fn a_file_that_never_parsed_asks_the_user_instead_of_failing_open() {
    let mut broken = FileState::new(Scope::Global, PathBuf::from("/x/policy.toml"));
    broken.error = Some(ParseError { line: 3, message: "expected `=`".into() });
    let s = who("front", true);
    let e = eval(&[&broken], &s, "ls", None);
    assert_eq!((e.effect, e.source), (Effect::User, Source::Default));
    // Even a call the defaults would allow, and one another file allows.
    let ok = file(Scope::Project("shop".into()), "[[rule]]\nid = \"a\"\nmatch = \"Bash(ls:*)\"\neffect = \"allow\"\n");
    assert_eq!(eval(&[&broken, &ok], &s, "ls", None).effect, Effect::User);
    // A file that parsed once keeps its last good rules instead.
    let mut kept = file(Scope::Global, "[[rule]]\nid = \"a\"\nmatch = \"Bash(ls:*)\"\neffect = \"allow\"\n");
    kept.error = broken.error.clone();
    assert_eq!(eval(&[&kept], &s, "ls", None).effect, Effect::Allow);
}

#[test]
fn a_commented_rule_is_valid_toml_once_uncommented() {
    let d = tempfile::Builder::new().prefix("pol").tempdir_in(orch_protocol::test_tmp()).unwrap();
    let path = d.path().join("policy.toml");
    let rule = Rule {
        id: "proposal-1".into(),
        pattern: "Bash(echo \"é\u{7f}\\ \t:*)".into(),
        effect: Effect::Allow,
        role: Some("front".into()),
        agent: None,
        from: Some("proposal:d-1".into()),
        added_ms: None,
        note: None,
    };
    file::append_commented_rule(&path, &rule).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let on: String = text.lines().map(|l| l.strip_prefix("# ").filter(|_| l.starts_with("# [[") || l.starts_with("# id") || l.starts_with("# match") || l.starts_with("# effect") || l.starts_with("# role") || l.starts_with("# from")).unwrap_or(l)).collect::<Vec<_>>().join("\n");
    let doc = file::parse(&on).expect("the uncommented rule parses");
    assert_eq!(doc.rules.len(), 1);
    assert_eq!(doc.rules[0].pattern, rule.pattern);
}
