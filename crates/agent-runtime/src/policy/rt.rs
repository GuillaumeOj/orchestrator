//! The policy in the Runtime: the files it knows (synced with the registered
//! projects), the hard lists' context for an agent, rule writes (Always
//! allow, `rule add` / `revoke`), the parse-error alerts, the one-time
//! export of the store's rules (migration 14) and the `policy_*` commands.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Result, anyhow, bail};
use orch_store::{InboxStatus, RuleEffect, RuleScope};
use serde_json::{Value, json};

use super::hard::{self, HardCtx, HardVerdict};
use super::{Defaults, Effect, Evaluation, Rule, Scope, file};
use crate::permissions::Subject;
use crate::runtime::Runtime;
use crate::store::AgentRecord;

/// The `alert` decision a broken policy file raises (its payload's `alert`).
pub const POLICY_ALERT: &str = "policy_parse";

impl Runtime {
    /// `<app data>/policy.toml`.
    pub fn global_policy_path(&self) -> PathBuf {
        self.cfg.paths.root.join("policy.toml")
    }

    /// `<project>/.orchestrator/policy.toml` when the project tracks
    /// `.orchestrator/`, else `<app data>/projects/<id>/policy.toml`.
    pub fn project_policy_path(&self, p: &orch_store::Project) -> PathBuf {
        if p.track_orchestrator_dir {
            p.path.join(".orchestrator").join("policy.toml")
        } else {
            self.cfg.paths.root.join("projects").join(&p.id).join("policy.toml")
        }
    }

    fn policy_path(&self, scope: &Scope) -> Result<PathBuf> {
        Ok(match scope {
            Scope::Global => self.global_policy_path(),
            Scope::Project(id) => {
                let p = self.db.project(id)?.ok_or_else(|| anyhow!("no project {id}"))?;
                self.project_policy_path(&p)
            }
        })
    }

    /// Brings the known files in line with the registered projects (new
    /// ones read, moved ones re-read, removed ones dropped) and the watcher
    /// with them. Cheap when nothing changed.
    pub(crate) fn policy_sync(&self) {
        let mut want = BTreeMap::from([(Scope::Global, self.global_policy_path())]);
        for p in self.db.projects(false).unwrap_or_default() {
            want.insert(Scope::Project(p.id.clone()), self.project_policy_path(&p));
        }
        for (scope, changed) in self.policy.set_paths(want) {
            if changed {
                self.policy_alert(&scope);
            }
        }
        self.policy_watch();
    }

    /// Reads `scope`'s file again (a watcher event, our own write).
    pub(crate) fn policy_reload(&self, scope: &Scope) {
        if self.policy.reload(scope) == Some(true) {
            self.policy_alert(scope);
        }
        self.touch("policy");
    }

    pub(crate) fn policy_reload_path(&self, path: &std::path::Path) {
        if let Some(scope) = self.policy.scope_of(path) {
            self.policy_reload(&scope);
        }
    }

    /// The file's parse error changed: one open `alert` per file, closed
    /// (stale) once it parses again. The last good rules stay in effect.
    fn policy_alert(&self, scope: &Scope) {
        let Some(f) = self.policy.file(scope) else { return };
        let path = f.path.display().to_string();
        let open: Vec<u64> = self.store.read(|s| {
            s.inbox
                .iter()
                .filter(|i| {
                    i.open()
                        && i.kind == "alert"
                        && i.payload.get("alert").and_then(Value::as_str) == Some(POLICY_ALERT)
                        && i.payload.get("file").and_then(Value::as_str) == Some(path.as_str())
                })
                .map(|i| i.id)
                .collect()
        });
        for id in &open {
            let _ = self.close_item(*id, InboxStatus::Stale, "fixed", "core");
        }
        if let Some(e) = &f.error {
            let body = format!(
                "{path} line {}: {}\nThe policy file does not parse: its last good version stays in effect until it is fixed.",
                e.line, e.message
            );
            let payload = json!({"alert": POLICY_ALERT, "file": path, "scope": scope.label(),
                                 "project": scope.project(), "line": e.line, "message": e.message});
            let _ = self.add_item(None, "alert", body, payload, 0);
        }
    }

    /// The hard lists' view of `rec` (spec §3.2.1).
    pub fn hard_ctx(&self, rec: &AgentRecord, cwd: Option<&str>) -> HardCtx {
        let mut protected = vec!["main".to_string(), "master".to_string()];
        if let Ok(Some(p)) = self.db.project_by_path(&rec.repo) {
            protected.push(p.default_branch.clone());
            protected.extend(p.typed_settings().base_branch);
        }
        // A specialist never merges into its run's integration branch (its lead does).
        if let Some(seat) = rec.team.as_ref().filter(|_| !crate::runtime::reserved_slot(rec)) {
            if let Ok(Some(f)) = self.db.feature(&seat.feature_id) {
                protected.push(f.integration_branch());
            }
        }
        protected.sort();
        protected.dedup();
        HardCtx {
            home: crate::paths::home_dir(),
            app_root: self.cfg.paths.root.clone(),
            worktree: rec.worktree.clone(),
            cwd: cwd.map(PathBuf::from).unwrap_or_else(|| rec.worktree.clone()),
            extra_write: rec.role.extra_write.clone(),
            network_allow: rec.role.network_allow.clone(),
            protected,
            branch: Default::default(),
        }
    }

    /// Is `rec`'s call on a hard list? (B2, B3: every Overwatch path checks it.)
    pub fn hard_check(&self, rec: &AgentRecord, tool: &str, input: &Value, cwd: Option<&str>) -> Option<HardVerdict> {
        hard::check(&self.hard_ctx(rec, cwd), tool, input)
    }

    /// The policy's answer for `rec`'s call, hard lists included.
    /// `use_defaults` false: no matching rule gives `None` (orchestrator tools run).
    pub fn evaluate_call(
        &self,
        rec: &AgentRecord,
        tool: &str,
        input: &Value,
        cwd: Option<&str>,
        use_defaults: bool,
    ) -> Option<Evaluation> {
        self.policy_sync();
        let hard = self.hard_check(rec, tool, input, cwd);
        self.policy.evaluate(&self.subject(rec), tool, input, hard.as_ref(), use_defaults)
    }

    /// The file a new rule goes to: project → its file; role → the
    /// project's file (`project` given) else the global one; agent → its
    /// project's file, else the global one.
    fn rule_target(
        &self,
        scope: RuleScope,
        scope_id: &str,
        project: Option<&str>,
    ) -> Result<(Scope, Option<String>, Option<String>)> {
        Ok(match scope {
            RuleScope::Project => {
                self.db.project(scope_id)?.ok_or_else(|| anyhow!("no project {scope_id}"))?;
                (Scope::Project(scope_id.into()), None, None)
            }
            RuleScope::Role => {
                (project.map_or(Scope::Global, |p| Scope::Project(p.into())), Some(scope_id.into()), None)
            }
            RuleScope::Agent => {
                let rec = self.store.read(|s| s.agents.get(scope_id).cloned());
                let p = project.map(str::to_string).or_else(|| rec.and_then(|r| self.subject(&r).project_id));
                (p.map_or(Scope::Global, Scope::Project), None, Some(scope_id.into()))
            }
        })
    }

    /// Adds a rule to a policy file (FR-1308, the FR-708 scopes mapped by
    /// [`Runtime::rule_target`]) and logs it (FR-709, actor `user`).
    /// `{rule, file, path}`.
    pub fn add_rule(
        &self,
        scope: RuleScope,
        scope_id: &str,
        project: Option<&str>,
        pattern: &str,
        effect: Effect,
        from: Option<String>,
    ) -> Result<Value> {
        let pattern = pattern.trim();
        if pattern.is_empty() {
            bail!("a rule needs a pattern");
        }
        self.policy_sync();
        let (target, role, agent) = self.rule_target(scope, scope_id, project)?;
        let path = self.policy_path(&target)?;
        let w = self.policy.write_lock();
        let rule = Rule {
            id: self.policy.next_id(),
            pattern: pattern.into(),
            effect,
            role,
            agent,
            from,
            added_ms: Some(orch_protocol::now_ms()),
            note: None,
        };
        file::append_rule(&path, &rule)?;
        self.policy_reload(&target);
        drop(w);
        let out = json!({"rule": rule, "file": target.label(), "path": path.display().to_string()});
        let mut detail = out.clone();
        (detail["id"], detail["pattern"], detail["effect"]) = (json!(rule.id), json!(pattern), json!(effect));
        self.db.audit("user", None, "rule_added", Some(pattern), effect.as_str(), detail)?;
        Ok(out)
    }

    /// Removes a rule from its file (`file`: the label, when ids repeat
    /// across files) and logs it. False if unknown.
    pub fn revoke_rule(&self, id: &str, file_label: Option<&str>) -> Result<bool> {
        self.policy_sync();
        let scope =
            file_label.map(|l| Scope::parse(l).ok_or_else(|| anyhow!("file: global | project:<id>"))).transpose()?;
        let w = self.policy.write_lock();
        let found = self.policy.find(id, scope.as_ref());
        let done = match &found {
            Some((sc, _)) => {
                let done = file::remove_rule(&self.policy_path(sc)?, id)?;
                self.policy_reload(sc);
                done
            }
            None => false,
        };
        drop(w);
        self.db.audit(
            "user",
            None,
            "rule_revoked",
            Some(id),
            if done { "ok" } else { "unknown" },
            json!({"rule": id, "file": found.as_ref().map(|(s, _)| s.label()),
                   "pattern": found.as_ref().map(|(_, r)| &r.pattern),
                   "effect": found.as_ref().map(|(_, r)| r.effect.as_str())}),
        )?;
        Ok(done)
    }

    /// Every rule of every file, global first: `[{file, path, rule…}]`.
    pub fn policy_rules(&self, project: Option<&str>) -> Vec<Value> {
        self.policy_sync();
        self.policy
            .files()
            .into_iter()
            .filter(|f| project.is_none() || f.scope == Scope::Global || f.scope.project() == project)
            .flat_map(|f| {
                let (label, path) = (f.scope.label(), f.path.display().to_string());
                f.doc.rules.into_iter().map(move |r| {
                    let mut v = json!(r);
                    v["file"] = json!(label);
                    v["path"] = json!(path);
                    v
                })
            })
            .collect()
    }

    /// `policy_files`: the global file and every registered project's, also
    /// when missing (api-contract §3.3, §5 F15).
    pub fn policy_files(&self) -> Vec<Value> {
        self.policy_sync();
        self.policy.files().iter().map(super::FileState::row).collect()
    }

    pub fn policy_defaults(&self) -> Defaults {
        self.policy_sync();
        self.policy.defaults()
    }

    /// Writes `[defaults]` in the global file (overwatch | user each).
    pub fn set_policy_defaults(&self, team: &str, solo: &str) -> Result<Defaults> {
        let parse = |k: &str, v: &str| match Effect::parse(v) {
            Some(e @ (Effect::Overwatch | Effect::User)) => Ok(e),
            _ => Err(anyhow!("{k}: overwatch | user")),
        };
        let d = Defaults { team: parse("team", team)?, solo: parse("solo", solo)? };
        self.policy_sync();
        {
            let _w = self.policy.write_lock();
            file::set_defaults(&self.global_policy_path(), &d)?;
            self.policy_reload(&Scope::Global);
        }
        self.db.audit("user", None, "policy_defaults", None, "ok", json!(d))?;
        Ok(self.policy.defaults())
    }

    /// `policy_open {scope, project?}`: the file's path, created (`version =
    /// 1` and a comment) when missing, for the editor (FR-1101).
    pub fn policy_open(&self, scope: &str, project: Option<&str>) -> Result<PathBuf> {
        let sc = match (scope, project) {
            ("global", _) => Scope::Global,
            ("project", Some(p)) => Scope::Project(p.into()),
            _ => bail!("scope: global | project (with project)"),
        };
        let path = self.policy_path(&sc)?;
        if file::ensure(&path)? {
            self.policy_sync();
            self.policy_reload(&sc);
        }
        Ok(path)
    }

    /// `orchd policy --test`: the effect for `tool` / `input` as `role` (a
    /// solo session when `solo`), and what decides it. The hard lists use the
    /// project's checkout as the worktree and no network allow-list.
    pub fn policy_test(&self, project: Option<&str>, role: &str, tool: &str, input: &Value) -> Result<Value> {
        self.policy_sync();
        let proj = project.map(|p| self.db.project(p)?.ok_or_else(|| anyhow!("no project {p}"))).transpose()?;
        let wt = proj.as_ref().map(|p| p.path.clone()).unwrap_or_else(|| PathBuf::from("/"));
        let ctx = HardCtx {
            home: crate::paths::home_dir(),
            app_root: self.cfg.paths.root.clone(),
            cwd: wt.clone(),
            worktree: wt,
            protected: ["main", "master"]
                .iter()
                .map(|s| s.to_string())
                .chain(proj.iter().map(|p| p.default_branch.clone()))
                .collect(),
            ..Default::default()
        };
        let hard = hard::check(&ctx, tool, input);
        let s = Subject {
            agent_id: String::new(),
            role: role.into(),
            project_id: proj.map(|p| p.id),
            team: role != "solo",
        };
        let ev = self.policy.evaluate(&s, tool, input, hard.as_ref(), true);
        Ok(json!({"tool": tool, "input": input, "role": role, "evaluation": ev, "hard": hard}))
    }

    /// Migration 14 (D8): every non-revoked store rule not yet exported is
    /// written to the file of its scope with `id = "r<row id>"` (`ask` →
    /// `user`), then marked. Idempotent: an id already in the file is not
    /// written twice. A rule whose project is gone is marked without export.
    pub(crate) fn migrate_store_rules(&self) {
        let rows = match self.db.rules_to_migrate() {
            Ok(r) if r.is_empty() => return,
            Ok(r) => r,
            Err(e) => return eprintln!("warn: store rules not read for migration: {e:#}"),
        };
        self.policy_sync();
        let _w = self.policy.write_lock();
        let mut done = vec![];
        for r in rows {
            let target = match r.scope {
                // Gone (not a read error: that one is retried at the next start).
                RuleScope::Project if matches!(self.db.project(&r.scope_id), Ok(None)) => {
                    done.push(r.id);
                    continue;
                }
                _ => self.rule_target(r.scope, &r.scope_id, None),
            };
            let Ok((scope, role, agent)) = target else { continue };
            let id = format!("r{}", r.id);
            let rule = Rule {
                id: id.clone(),
                pattern: r.pattern.clone(),
                effect: match r.effect {
                    RuleEffect::Allow => Effect::Allow,
                    RuleEffect::Ask => Effect::User,
                    RuleEffect::Deny => Effect::Deny,
                },
                role,
                agent,
                from: Some("store".into()),
                added_ms: Some(r.created_at_ms),
                note: None,
            };
            let written = if self.policy.find(&id, Some(&scope)).is_some() {
                Ok(())
            } else {
                self.policy_path(&scope).and_then(|p| file::append_rule(&p, &rule))
            };
            match written {
                Ok(()) => {
                    self.policy_reload(&scope);
                    done.push(r.id);
                }
                Err(e) => eprintln!("warn: store rule {} not migrated: {e:#}", r.id),
            }
        }
        if let Err(e) = self.db.mark_rules_migrated(&done) {
            eprintln!("warn: migrated store rules not marked: {e:#}");
        } else if !done.is_empty() {
            let _ = self.db.audit("core", None, "rules_migrated", None, "ok", json!({"rules": done}));
        }
    }
}
