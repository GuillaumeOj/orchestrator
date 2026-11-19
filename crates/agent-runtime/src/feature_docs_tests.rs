//! A team run with each Feature docs setting (change-how-specs-are-saved
//! spec §5): the folders the lead is told, the gate, the sandbox grant,
//! hand-offs and deletion.

use serde_json::{Value, json};

use crate::runtime::Runtime;
use crate::team::LaunchSpec;
use crate::team::tests::{call, ok, queued, setup, write_commit};

fn set_docs(rt: &Runtime, pid: &str, v: Value) {
    let s = orch_team::FeatureDocsSetting::from_form(&v).unwrap();
    rt.db
        .update_project_settings(pid, move |e| {
            e.settings.feature_docs = s;
            Ok(())
        })
        .unwrap();
}

fn launch(rt: &Runtime, pid: &str) -> orch_store::TeamRun {
    rt.launch(&LaunchSpec {
        project_id: pid.into(),
        team_id: "bugfix-squad".into(),
        slug: "fx".into(),
        title: "Fix X".into(),
        brief: "Fix the X bug.".into(),
        ..Default::default()
    })
    .unwrap()
}

/// The system prompt a spawn of `agent` would get, and its settings.
fn compiled(rt: &Runtime, agent: &str) -> (String, Value) {
    let rec = rt.store.read(|s| s.agents[agent].clone());
    let c = rt.compile_for_test(&rec).unwrap();
    let i = c.args.iter().position(|a| a == "--append-system-prompt").unwrap();
    (c.args[i + 1].clone(), c.settings)
}

fn gate(rt: &Runtime) -> orch_store::InboxItem {
    rt.store.read(|s| s.inbox.iter().find(|i| i.pending() && i.kind == "gate").cloned().unwrap())
}

#[test]
fn the_default_is_the_legacy_folder() {
    let (_d, rt, _rd, _repo, pid) = setup();
    let run = launch(&rt, &pid);
    let f = rt.db.feature(&run.feature_id).unwrap().unwrap();
    assert_eq!(f.docs_location, Some(orch_team::DocsLocation::legacy("fx")), "snapshot of the default");
    let lead = "fx-lead-1";
    assert!(queued(&rt, lead)[0].contains("write the docs in .orchestrator/features/fx/, commit them, then"));
    let (prompt, settings) = compiled(&rt, lead);
    assert!(prompt.contains("Feature docs: .orchestrator/features/fx/ on that branch.\nYou are node"));
    assert!(!prompt.contains("## Feature folders") && !prompt.contains("Designs folder"));
    let root = crate::compile::rule_path(&rt.cfg.paths.root);
    assert!(
        settings["permissions"]["deny"].as_array().unwrap().contains(&json!(format!("Edit({root}/**)"))),
        "{settings}"
    );
    assert!(!rt.cfg.paths.root.join("projects").exists(), "no app folder");
}

#[test]
fn a_custom_repo_folder_and_a_separate_designs_folder() {
    let (_d, rt, _rd, _repo, pid) = setup();
    set_docs(
        &rt,
        &pid,
        json!({"docs": {"kind": "repo", "path": "docs/specs/{slug}"}, "designs": {"kind": "repo", "path": "docs/design/{slug}"}}),
    );
    let run = launch(&rt, &pid);
    let lead = "fx-lead-1";
    assert!(queued(&rt, lead)[0].contains("write the docs in docs/specs/fx/, commit them"));
    let (prompt, _) = compiled(&rt, lead);
    assert!(prompt.contains("Feature docs: docs/specs/fx/ on that branch."), "{prompt}");
    assert!(prompt.contains("Designs folder: docs/design/fx/ on that branch"));
    assert!(prompt.contains("Write `docs/specs/fx/spec.md`"));

    // §3.2: a setting changed while the feature runs leaves its folders.
    set_docs(&rt, &pid, json!({"docs": {"kind": "app"}, "designs": {"kind": "with_docs"}}));
    let f = rt.db.feature(&run.feature_id).unwrap().unwrap();
    assert_eq!(f.docs_location.as_ref().unwrap().docs, orch_team::Loc::repo("docs/specs/fx"));

    write_commit(&rt, lead, "docs/specs/fx/spec.md", "# Fix X\n![a](../../design/fx/a.png)\n");
    write_commit(&rt, lead, "docs/design/fx/a.png", "\u{89}PNG");
    ok(&rt, lead, "request_spec_approval", json!({"summary": "s"}));
    assert_eq!(gate(&rt).payload["doc_lines"], json!([{"path": "docs/specs/fx/spec.md", "lines": 2}]));
    let v = rt.run_json(&run.id).unwrap();
    assert_eq!(v["docs"], json!(["spec.md"]));
    assert_eq!(
        v["docs_location"],
        json!({"docs": {"kind": "repo", "dir": "docs/specs/fx"}, "designs": {"kind": "repo", "dir": "docs/design/fx"}})
    );
    assert!(rt.feature_asset(&run.feature_id, "../../design/fx/a.png").is_ok(), "the doc's link previews");
    assert!(rt.feature_docs_app_dir(&run.feature_id, "docs").unwrap_err().to_string().contains("in the repository"));
}

#[test]
fn docs_not_saved_in_the_repository() {
    let (_d, rt, _rd, _repo, pid) = setup();
    set_docs(&rt, &pid, json!({"docs": {"kind": "app"}, "designs": {"kind": "with_docs"}}));
    let run = launch(&rt, &pid);
    let lead = "fx-lead-1";
    let base = crate::feature_docs::app_base(&rt.cfg.paths.root, &pid, "fx");
    let (docs, design) = (base.join("docs"), base.join("docs/design"));
    assert!(docs.is_dir() && design.is_dir(), "created at launch");
    assert!(queued(&rt, lead)[0].contains(&format!("write the docs in {}/, do not commit them", docs.display())));

    // Every agent of the run: read & write in its sandbox, a prompt line;
    // the rest of the app's data stays denied.
    let (prompt, settings) = compiled(&rt, lead);
    assert!(prompt.contains("## Feature folders") && prompt.contains(&format!("- `{}/`", docs.display())));
    assert!(prompt.contains("Do not commit them"), "{prompt}");
    let fs = &settings["sandbox"]["filesystem"];
    assert!(fs["allowWrite"].as_array().unwrap().contains(&json!(docs.display().to_string())));
    let deny: Vec<&str> =
        settings["permissions"]["deny"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    let root = crate::compile::rule_path(&rt.cfg.paths.root);
    assert!(!deny.contains(&format!("Edit({root}/**)").as_str()), "the whole app data is no longer denied");
    assert!(deny.iter().any(|r| r.starts_with(&format!("Edit({root}/state"))), "{deny:?}");
    let docs_rule = crate::compile::rule_path(&docs);
    assert!(
        !deny.contains(&format!("Edit({docs_rule}/**)").as_str())
            && !deny.contains(&format!("Edit({docs_rule})").as_str())
    );
    assert!(deny.contains(&format!("Edit({docs_rule}/**/.claude/**)").as_str()), "code run outside stays unwritable");
    let rec = rt.store.read(|s| s.agents[lead].clone());
    let write = |p: &std::path::Path| rt.hard_check(&rec, "Write", &json!({"file_path": p, "content": "x"}), None);
    assert!(write(&docs.join("spec.md")).is_none(), "the hard policy lets the feature folder through");
    assert!(write(&base.join("other.md")).is_some() && write(&rt.cfg.paths.root.join("x")).is_some());

    // The gate opens with no commit; nothing reaches the integration branch.
    std::fs::write(docs.join("spec.md"), "# Fix X\n").unwrap();
    ok(&rt, lead, "request_spec_approval", json!({"summary": "s"}));
    let g = gate(&rt);
    assert_eq!(g.payload["docs"], json!(["spec.md"]));
    assert_eq!(g.payload["doc_lines"], json!([{"path": format!("{}/spec.md", docs.display()), "lines": 1}]));
    let tree = orch_git::git(&rec.worktree, &["ls-tree", "-r", "--name-only", "HEAD"]).unwrap();
    assert!(!tree.contains("spec.md"), "{tree}");
    assert_eq!(rt.feature_doc(&run.feature_id, "spec.md").unwrap(), "# Fix X\n");

    // Designs: an absolute path only inside the feature's folders.
    std::fs::write(design.join("a.png"), "PNG").unwrap();
    let a = design.join("a.png").display().to_string();
    ok(&rt, lead, "handoff", json!({"artifact": a, "to": "back", "kind": "design"}));
    let art = rt.db.artifacts(&run.feature_id).unwrap().pop().unwrap();
    assert_eq!(art.meta.as_ref().unwrap().path.as_deref(), Some(a.as_str()));
    let outside = rt.cfg.paths.root.join("x.png");
    std::fs::write(&outside, "PNG").unwrap();
    let (err, text) = call(&rt, lead, "handoff", json!({"artifact": outside, "to": "back", "kind": "design"}));
    assert!(err && text.contains("must be an existing file in"), "{text}");
    let escape = format!("{}/../../x.png", design.display());
    assert!(call(&rt, lead, "handoff", json!({"artifact": escape, "to": "back", "kind": "design"})).0);
    assert_eq!(rt.feature_docs_app_dir(&run.feature_id, "designs").unwrap(), design);

    // Deleting the feature removes the folder.
    rt.delete_feature(&run.feature_id, false, false).unwrap();
    assert!(!base.exists());
}

#[test]
fn designs_alone_not_saved() {
    let (_d, rt, _rd, _repo, pid) = setup();
    set_docs(&rt, &pid, json!({"docs": {"kind": "repo", "path": "docs/{slug}"}, "designs": {"kind": "app"}}));
    launch(&rt, &pid);
    let design = crate::feature_docs::app_base(&rt.cfg.paths.root, &pid, "fx").join("design");
    let (prompt, settings) = compiled(&rt, "fx-lead-1");
    assert!(prompt.contains("Feature docs: docs/fx/ on that branch."));
    assert!(prompt.contains(&format!("Designs folder: {}/ (not saved in the repository)", design.display())));
    assert!(prompt.contains("Write `docs/fx/spec.md`") && prompt.contains("commit them with mcp__orch__commit"));
    assert!(settings["sandbox"]["filesystem"]["allowWrite"].as_array().unwrap().contains(&json!(design)));
}
