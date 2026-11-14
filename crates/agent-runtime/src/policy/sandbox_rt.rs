//! Sandbox allowances in the Runtime (api-contract §2–4): the guard built
//! from the registered projects, an agent's allowances (compile and the
//! hard list), the `sandbox_*` commands, and what a change does to running
//! agents.

use std::path::PathBuf;

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};

use super::sandbox::{Allowances, Guard, GuardProject, SandboxEntry, SandboxKind};
use super::{Scope, file};
use crate::role::DEFAULT_NETWORK_ALLOW;
use crate::runtime::Runtime;
use crate::state::AgentState;
use crate::store::AgentRecord;

/// Whether the CLI applies a rewritten `--settings` file to a running
/// session. Claude Code 2.1.325 reloads `sandbox.network.allowLocalBinding`
/// from user settings live, but a reload of the `--settings` file itself is
/// not verified (README, "Sandbox allowances"): changes apply at the next start.
pub const SETTINGS_RELOAD_LIVE: bool = false;

/// The now topic a sandbox change touches (`sandbox_changed`, api-contract §4).
pub const SANDBOX_TOPIC: &str = "sandbox";

fn applies_label() -> &'static str {
    if SETTINGS_RELOAD_LIVE { "live" } else { "next_start" }
}

fn kind_of(kind: &str) -> Result<SandboxKind> {
    SandboxKind::parse(kind).ok_or_else(|| anyhow!("kind: network | write | local_ports | unix_socket"))
}

/// The UI's `{scope, project?}`, or a file label.
fn scope_of(scope: &str, project: Option<&str>) -> Result<Scope> {
    match (scope, project.filter(|p| !p.is_empty())) {
        ("global", _) => Ok(Scope::Global),
        ("project", Some(p)) => Ok(Scope::Project(p.into())),
        ("project", None) => bail!("scope project needs a project"),
        (l, _) => Scope::parse(l).ok_or_else(|| anyhow!("scope: global | project")),
    }
}

fn entry_json(e: &SandboxEntry) -> Value {
    let mut v = json!(e);
    v["source"] = json!("file");
    v
}

impl Runtime {
    /// What no allowance may touch, from the registered `projects` (built
    /// at each [`Runtime::policy_sync`]; the rest reads [`super::Policy::guard`]).
    pub(crate) fn sandbox_guard(&self, projects: &[orch_store::Project]) -> Guard {
        let projects = projects
            .iter()
            .map(|p| GuardProject {
                id: p.id.clone(),
                checkout: p.path.clone(),
                worktree_root: p.worktree_root.clone(),
            })
            .collect();
        let ssh = std::env::var_os("SSH_AUTH_SOCK").filter(|s| !s.is_empty()).map(PathBuf::from);
        let mut g = Guard::new(&crate::paths::home_dir(), &self.cfg.paths.root, projects, ssh);
        // The login PATH's folders (`~/.cargo/bin`, `~/go/bin`, …): what is
        // written there runs outside the sandbox the next time the user types its name.
        let on_path = std::env::split_paths(&self.login_path)
            .filter(|p| p.is_absolute() && p.parent().is_some())
            .map(|p| super::sandbox::real_lenient(&p));
        for p in on_path {
            super::sandbox::push_new(&mut g.unsandboxed, p);
        }
        g
    }

    /// The current guard, the files read first when they never were.
    fn guard(&self, project: Option<&str>) -> Guard {
        self.policy_ensure(project);
        self.policy.guard()
    }

    /// `rec`'s allowances (global and `project`'s files, its role),
    /// re-validated now. The Overwatch has none.
    pub(crate) fn allowances(&self, rec: &AgentRecord, project: Option<&str>) -> Allowances {
        if rec.is_overwatch() {
            return Allowances::default();
        }
        let g = self.guard(project);
        Allowances::of(&self.policy.sandbox_entries(project), &rec.role.name, &g, project)
    }

    /// The hosts `rec` may reach: its role's, then the network allowances
    /// (the hard list's `network_allow`; no path work, the hot path).
    pub(crate) fn network_allow(&self, rec: &AgentRecord, project: Option<&str>) -> Vec<String> {
        if rec.is_overwatch() {
            return rec.role.network_allow.clone();
        }
        self.policy_ensure(project);
        self.policy.sandbox_hosts(project, &rec.role.name, &rec.role.network_allow)
    }

    /// The running agents (not the Overwatch) of `project` (`None`: all).
    pub(crate) fn live_in_scope(&self, project: Option<&str>) -> Vec<AgentRecord> {
        self.live_ids()
            .iter()
            .filter_map(|id| self.store.read(|s| s.agents.get(id).cloned()))
            .filter(|rec| !rec.is_overwatch())
            .filter(|rec| project.is_none_or(|p| self.agent_project(rec).as_deref() == Some(p)))
            .collect()
    }

    /// `sandbox_kinds`: what the installed CLI supports.
    pub fn sandbox_kinds(&self) -> Vec<&'static str> {
        SandboxKind::SUPPORTED.iter().map(|k| k.as_str()).collect()
    }

    /// `sandbox_check`: `{ok: true, value}` | `{ok: false, reason}`, no write.
    pub fn sandbox_check(&self, kind: &str, value: Option<&str>, scope: &str, project: Option<&str>) -> Result<Value> {
        let kind = kind_of(kind)?;
        let sc = scope_of(scope, project)?;
        Ok(match super::sandbox::validate(kind, value, &self.guard(sc.project()), sc.project()) {
            Ok(v) => json!({"ok": true, "value": v}),
            Err(reason) => json!({"ok": false, "reason": reason}),
        })
    }

    /// Live agents a change of `scope`'s file (for `role`) reaches.
    fn sandbox_running(&self, scope: &Scope, role: Option<&str>) -> usize {
        self.live_in_scope(scope.project()).iter().filter(|rec| role.is_none_or(|r| r == rec.role.name)).count()
    }

    /// `sandbox_add`: validates, appends to the file of `scope` (a duplicate
    /// returns the existing entry), logs it (actor `user`).
    /// `{entry, file, path, applies, running, created}`.
    pub fn sandbox_add(
        &self,
        scope: &str,
        project: Option<&str>,
        kind: &str,
        value: Option<&str>,
        role: Option<&str>,
        note: Option<&str>,
    ) -> Result<Value> {
        let kind = kind_of(kind)?;
        let sc = scope_of(scope, project)?;
        self.policy_sync();
        if !self.policy.knows(&sc) {
            bail!("no project {}", sc.project().unwrap_or(""));
        }
        let value =
            super::sandbox::validate(kind, value, &self.policy.guard(), sc.project()).map_err(|r| anyhow!(r))?;
        let (role, note) = super::form_role_note(role, note)?;
        let path = self.policy_path(&sc)?;
        let w = self.policy.write_lock();
        let existing = self
            .policy
            .file(&sc)
            .and_then(|f| f.doc.sandbox.into_iter().find(|e| e.same(kind, value.as_deref(), role.as_deref())));
        let (entry, created) = match existing {
            Some(e) => (e, false),
            None => {
                let e = SandboxEntry {
                    id: self.policy.next_sandbox_id(),
                    kind,
                    value,
                    role,
                    note,
                    added_ms: Some(orch_protocol::now_ms()),
                    invalid: None,
                };
                file::append_sandbox(&path, &e)?;
                self.policy_reload(&sc);
                (e, true)
            }
        };
        drop(w);
        let out = json!({
            "entry": entry_json(&entry), "file": sc.label(), "path": path.display().to_string(),
            "applies": applies_label(), "running": self.sandbox_running(&sc, entry.role.as_deref()), "created": created,
        });
        if created {
            // In the file already: a failed log line is no failure of the add.
            let target = entry.value.clone().unwrap_or_else(|| kind.as_str().into());
            if let Err(e) = self.db.audit("user", None, "sandbox_add", Some(&target), "ok", out.clone()) {
                eprintln!("warn: sandbox entry {} added but not logged: {e:#}", entry.id);
            }
        }
        Ok(out)
    }

    /// `sandbox_remove {id, file}`: `{removed, applies, running}`; logged.
    pub fn sandbox_remove(&self, id: &str, file_label: &str) -> Result<Value> {
        let sc = Scope::parse(file_label).ok_or_else(|| anyhow!("file: global | project:<id>"))?;
        self.policy_sync();
        let path = self.policy_path(&sc)?;
        let w = self.policy.write_lock();
        let found = self.policy.file(&sc).and_then(|f| f.doc.sandbox.into_iter().find(|e| e.id == id));
        let Some(entry) = found else { bail!("no sandbox entry {id} in {file_label}") };
        if !file::remove_sandbox(&path, id)? {
            bail!("no sandbox entry {id} in {}", path.display());
        }
        self.policy_reload(&sc);
        drop(w);
        let out = json!({"removed": entry_json(&entry), "file": sc.label(), "applies": applies_label(),
                         "running": self.sandbox_running(&sc, entry.role.as_deref())});
        // Out of the file already: a failed log line is no failure of the remove.
        if let Err(e) =
            self.db.audit("user", None, "sandbox_remove", entry.value.as_deref().or(Some(id)), "ok", out.clone())
        {
            eprintln!("warn: sandbox entry {id} removed but not logged: {e:#}");
        }
        Ok(out)
    }

    /// A file's `[[sandbox]]` changed (an add, a remove, a hand edit): the
    /// open settings reload, running agents in scope get their settings rewritten.
    pub(crate) fn sandbox_changed(&self, scope: &Scope) {
        self.touch(SANDBOX_TOPIC);
        self.rewrite_live_settings(scope.project());
    }

    /// The network hosts of the roles of `project`'s teams, beyond the
    /// core's: `[(role id, host)]`.
    fn role_hosts(&self, project: &str) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = self.store.read(|s| {
            s.agents
                .values()
                .filter(|a| a.tracker.state != AgentState::Archived)
                .filter(|a| a.team.as_ref().is_some_and(|t| t.project_id == project))
                .flat_map(|a| {
                    a.role
                        .network_allow
                        .iter()
                        .filter(|h| !DEFAULT_NETWORK_ALLOW.contains(&h.as_str()))
                        .map(|h| (a.role.name.clone(), h.clone()))
                        .collect::<Vec<_>>()
                })
                .collect()
        });
        out.sort();
        out.dedup();
        out
    }

    /// `sandbox_list {project?}`: the global scope, then each project's
    /// (only `project`'s when given), with read-only defaults.
    pub fn sandbox_list(&self, project: Option<&str>) -> Vec<Value> {
        self.policy_sync();
        self.policy
            .files()
            .into_iter()
            .filter(|f| project.is_none() || f.scope == Scope::Global || f.scope.project() == project)
            .map(|f| {
                let mut v = f.row();
                for k in ["rules", "sandbox"] {
                    v.as_object_mut().map(|o| o.remove(k));
                }
                v["entries"] =
                    json!(f.doc.sandbox.iter().filter(|e| e.invalid.is_none()).map(entry_json).collect::<Vec<_>>());
                v["defaults"] = match f.scope.project() {
                    None => json!(
                        DEFAULT_NETWORK_ALLOW
                            .iter()
                            .map(|h| json!({"kind": "network", "value": h, "source": "core"}))
                            .collect::<Vec<_>>()
                    ),
                    Some(p) => json!(self
                        .role_hosts(p)
                        .into_iter()
                        .map(|(r, h)| json!({"kind": "network", "value": h, "source": format!("role:{r}"), "role": r}))
                        .collect::<Vec<_>>()),
                };
                v
            })
            .collect()
    }
}
