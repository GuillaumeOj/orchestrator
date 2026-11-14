//! An agent asks for sandbox access (api-contract §5): the
//! `request_sandbox_access` tool, the `sandbox` Inbox card it parks on, the
//! user's decision, and the restart that applies an approval.

use anyhow::{Result, anyhow, bail};
use orch_protocol::InboxDecision;
use serde_json::{Value, json};

use super::sandbox::{self, Allowances, SandboxKind};
use super::sandbox_rt::SETTINGS_RELOAD_LIVE;
use crate::inbox::{PRIORITY_BLOCKING, Stage};
use crate::runtime::Runtime;
use crate::state::AgentState;
use crate::store::AgentRecord;
use orch_store::{InboxItem, InboxStatus};

/// The Inbox kind of a request.
pub const KIND: &str = "sandbox";
/// `why` is refused past this (the tool's schema says so too): it becomes
/// the entry's note, which holds as much.
pub const WHY_MAX: usize = crate::project_files::MAX_NOTE;

pub(crate) const ASKED: &str = "ORCH: asked the user; stop your turn and wait";
pub(crate) const ALREADY_ASKED: &str = "already asked; stop and wait";
pub(crate) const ALREADY_ALLOWED: &str = "already allowed";
pub(crate) const RESTART_TO_APPLY: &str = "; it applies when you restart: stop your turn";

/// "network api.github.com", "local ports": what the user and the agent read.
pub fn label(kind: SandboxKind, value: Option<&str>) -> String {
    let k = kind.as_str().replace('_', " ");
    match value {
        Some(v) if kind.has_value() => format!("{k} {v}"),
        _ => k,
    }
}

/// The message an approval (or several, coalesced) restarts the agent with.
pub fn approved_message(labels: &[String]) -> String {
    let verb = if labels.len() > 1 { "are" } else { "is" };
    format!("ORCH: approved: {} {verb} now allowed by the sandbox. Retry.", labels.join(", "))
}

/// `host` is `allowed` or under one of its `*.` wildcards.
fn host_covered(allowed: &[String], host: &str) -> bool {
    allowed.iter().any(|a| {
        a == host
            || a.strip_prefix("*.").is_some_and(|suffix| host.strip_suffix(suffix).is_some_and(|h| h.ends_with('.')))
    })
}

/// `allow` (plus the `role`'s own hosts and write folders) already grants
/// `kind` `value` (`path`: the value's canonical path for write and socket).
fn covered(
    allow: &Allowances,
    role: &crate::role::Role,
    kind: SandboxKind,
    value: Option<&str>,
    path: Option<&std::path::Path>,
) -> bool {
    match kind {
        SandboxKind::Network => host_covered(&allow.network_with(&role.network_allow), value.unwrap_or("")),
        SandboxKind::LocalPorts => allow.local_binding,
        SandboxKind::Write => path.is_some_and(|p| {
            allow.write.iter().any(|w| p.starts_with(w))
                || role.extra_write.iter().any(|w| p.starts_with(sandbox::real_lenient(w)))
        }),
        SandboxKind::UnixSocket => path.is_some_and(|p| allow.unix_sockets.iter().any(|s| s == p)),
    }
}

fn str_arg<'a>(args: &'a Value, k: &str) -> Option<&'a str> {
    args.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

fn same_request(i: &InboxItem, agent: &str, kind: SandboxKind, value: Option<&str>) -> bool {
    i.kind == KIND
        && i.open()
        && i.agent_id.as_deref() == Some(agent)
        && i.payload.get("kind").and_then(Value::as_str) == Some(kind.as_str())
        && i.payload.get("value").and_then(Value::as_str) == value
}

impl Runtime {
    /// `request_sandbox_access {kind, value?, why}` (api-contract §5): the
    /// text for the agent, or the reason it can never be allowed (an error).
    pub(crate) fn request_sandbox_access(&self, rec: &AgentRecord, args: &Value) -> Result<(String, Value)> {
        let kind = str_arg(args, "kind")
            .and_then(SandboxKind::parse)
            .ok_or_else(|| anyhow!("kind: network | write | local_ports | unix_socket"))?;
        if !SandboxKind::SUPPORTED.contains(&kind) {
            bail!("{} is not supported by the installed Claude Code", kind.as_str());
        }
        let why = str_arg(args, "why").ok_or_else(|| anyhow!("why: say what you need it for"))?;
        if why.chars().count() > WHY_MAX {
            bail!("why: at most {WHY_MAX} characters");
        }
        let project = self.agent_project(rec);
        self.policy_ensure(project.as_deref());
        let guard = self.policy.guard();
        let value =
            sandbox::validate(kind, str_arg(args, "value"), &guard, project.as_deref()).map_err(|r| anyhow!(r))?;
        let path = match value.as_deref() {
            Some(v) => sandbox::resolve(kind, v, &guard, project.as_deref()).map_err(|r| anyhow!(r))?,
            None => None,
        };
        let what = label(kind, value.as_deref());
        // Already in its effective sandbox: maybe not in the settings it runs on yet.
        let effective = self.allowances(rec, project.as_deref());
        if covered(&effective, &rec.role, kind, value.as_deref(), path.as_deref()) {
            let running = self.live_get(&rec.id).map(|l| l.sandbox.clone());
            let stale = !SETTINGS_RELOAD_LIVE
                && running.is_some_and(|r| !covered(&r, &rec.role, kind, value.as_deref(), path.as_deref()));
            if stale {
                self.sandbox_restart_due(&rec.id, what);
                return Ok((format!("{ALREADY_ALLOWED}{RESTART_TO_APPLY}"), json!({"allowed": true, "restart": true})));
            }
            return Ok((ALREADY_ALLOWED.into(), json!({"allowed": true})));
        }
        if let Some(id) =
            self.store.read(|s| s.inbox.iter().find(|i| same_request(i, &rec.id, kind, value.as_deref())).map(|i| i.id))
        {
            return Ok((ALREADY_ASKED.into(), json!({"item": id})));
        }
        let payload = json!({
            "kind": kind.as_str(), "value": value, "why": why, "project": project,
            "role": rec.role.name, "agent": rec.id,
        });
        let body = format!("{}: sandbox access: {what}", rec.id);
        let id = self.add_item(Some(&rec.id), KIND, body, payload, PRIORITY_BLOCKING)?;
        // No hold: parked at once, so its Stop frees the slot (FR-705) and
        // the idle logic does not resume it for other work.
        if let Ok(true) = self.store.write(|s| s.park_inbox(id))
            && let Some(i) = self.store.read(|s| s.inbox.iter().find(|i| i.id == id).cloned())
        {
            self.outcome(&i, "core", "parked");
        }
        let _ = self.db.audit(
            &format!("agent:{}", rec.id),
            Some(&rec.id),
            "sandbox_request",
            value.as_deref().or(Some(kind.as_str())),
            "asked",
            json!({"item": id, "kind": kind.as_str(), "value": value, "why": why}),
        );
        Ok((ASKED.into(), json!({"item": id})))
    }

    /// An approved allowance of `agent` waits for a restart: at its Stop,
    /// or now when it is not in a turn. Several coalesce into one restart.
    pub(crate) fn sandbox_restart_due(&self, agent: &str, what: String) {
        {
            let mut h = self.house();
            let due = h.sandbox_restart.entry(agent.to_string()).or_default();
            if !due.contains(&what) {
                due.push(what);
            }
        }
        let busy = self.live_get(agent).is_some()
            && self.store.read(|s| s.agents.get(agent).is_some_and(|a| a.tracker.state != AgentState::Idle));
        if !busy {
            self.sandbox_restart_now(agent);
        }
    }

    /// The agent's turn ended (Stop): restarts it on its approved allowances.
    pub(crate) fn sandbox_restart_at_stop(&self, agent: &str) {
        if self.house().sandbox_restart.contains_key(agent) {
            self.sandbox_restart_now(agent);
        }
    }

    /// Restarts `agent` with `--resume` on recompiled settings (a stopped
    /// one starts on them anyway) and tells it what is now allowed.
    fn sandbox_restart_now(&self, agent: &str) {
        let Some(labels) = self.house().sandbox_restart.remove(agent) else { return };
        // The stop intent is set first: the pump pastes nothing meanwhile,
        // and the turn goes to the respawned process.
        let restarted = !SETTINGS_RELOAD_LIVE && self.suspend_in_background(agent, "sandbox");
        self.event(agent, "sandbox_restart", json!({"allowances": labels, "restarted": restarted}));
        if let Err(e) = self.deliver_user(agent, &approved_message(&labels)) {
            eprintln!("warn: {agent} not told its sandbox allowances: {e:#}");
        }
    }

    /// The user's decision on a `sandbox` item (api-contract §5). Only the
    /// user decides one (never the Overwatch).
    pub(crate) fn decide_sandbox(
        &self,
        item: InboxItem,
        decision: InboxDecision,
        scope: Option<&str>,
        message: Option<&str>,
        by: &str,
    ) -> Result<Value> {
        let id = item.id;
        if by != "user" {
            bail!("only the user decides sandbox access");
        }
        let p = &item.payload;
        let kind = p.get("kind").and_then(Value::as_str).and_then(SandboxKind::parse);
        let kind = kind.ok_or_else(|| anyhow!("item {id} has no sandbox kind"))?;
        let value = p.get("value").and_then(Value::as_str).map(str::to_string);
        let project = p.get("project").and_then(Value::as_str).map(str::to_string);
        let role = p.get("role").and_then(Value::as_str).map(str::to_string);
        let why = p.get("why").and_then(Value::as_str).map(str::to_string);
        let what = label(kind, value.as_deref());
        // Checked before claiming: an Approve that can't be written is refused whole.
        let add = match decision {
            InboxDecision::Approve => {
                let sc = scope.unwrap_or("project");
                let (file_scope, add_role) = match sc {
                    "project" => ("project", None),
                    "global" => ("global", None),
                    "role" => ("project", role.clone()),
                    _ => bail!("scope: project | global | role"),
                };
                if file_scope == "project" && project.is_none() {
                    bail!("the agent has no project: allow it globally");
                }
                self.policy_ensure(project.as_deref());
                let checked = sandbox::validate(kind, value.as_deref(), &self.policy.guard(), project.as_deref());
                if let Err(reason) = checked {
                    let _ = self.store.write(|s| {
                        if let Some(i) = s.inbox.iter_mut().find(|i| i.id == id) {
                            i.payload["invalid"] = json!(reason);
                        }
                    });
                    bail!("{reason}");
                }
                Some((sc, file_scope, add_role))
            }
            InboxDecision::AlwaysAllow => bail!("approve it with a scope instead"),
            _ => None,
        };
        let word = orch_store::tag(&decision);
        let status = if decision == InboxDecision::Dismiss { InboxStatus::Dismissed } else { InboxStatus::Decided };
        let was = item.status;
        self.store
            .write(|s| {
                s.close_inbox(id, status, Some(word.clone()), by).map(|i| {
                    if let Some(m) = message {
                        i.payload["reply"] = json!(m);
                    }
                })
            })?
            .map_err(|now| anyhow!("Decision {id} is already {now:?}"))?;
        let mut result = json!({"item": id, "decision": word, "decided_by": by});
        let agent = item.agent_id.as_deref();
        let effect = match add {
            Some((sc, file_scope, add_role)) => {
                let added = self.sandbox_add(
                    file_scope,
                    project.as_deref(),
                    kind.as_str(),
                    value.as_deref(),
                    add_role.as_deref(),
                    why.as_deref(),
                );
                let added = match added {
                    Ok(a) => a,
                    Err(e) => {
                        self.unclaim(id, was);
                        return Err(e);
                    }
                };
                let mut entry = added["entry"].clone();
                entry["file"] = added["file"].clone();
                let outcome = json!({"entry": entry, "created": added["created"], "scope": sc});
                let _ = self.store.write(|s| {
                    if let Some(i) = s.inbox.iter_mut().find(|i| i.id == id) {
                        i.payload["outcome"] = outcome.clone();
                        i.payload.as_object_mut().map(|o| o.remove("invalid"));
                    }
                });
                result["outcome"] = outcome;
                result["sandbox"] = added;
                if let Some(a) = agent {
                    self.sandbox_restart_due(a, what.clone());
                }
                "allowed"
            }
            None if decision == InboxDecision::Dismiss => {
                if let Some(a) = agent {
                    let _ = self.deliver_user(
                        a,
                        &format!("ORCH: the user dismissed your sandbox access request ({what}). Continue without it."),
                    );
                }
                "dismissed"
            }
            None => {
                if let Some(a) = agent {
                    let mut text = format!("ORCH: denied: sandbox access to {what}.");
                    if let Some(m) = message.filter(|m| !m.trim().is_empty()) {
                        text.push_str(&format!(" The user said: {m}"));
                    }
                    // Nobody was told: the agent still waits, so the item does too.
                    match self.deliver_user(a, &text) {
                        Ok(turn) => result["turn"] = json!(turn),
                        Err(e) => {
                            self.unclaim(id, was);
                            return Err(e);
                        }
                    }
                }
                "denied"
            }
        };
        // After the effect (it carries the outcome): a failed log line must
        // not leave the item decided with no lifecycle.
        let logged = self.db.audit(
            by,
            agent,
            KIND,
            value.as_deref().or(Some(kind.as_str())),
            &word,
            json!({"item": id, "body": item.body, "message": message, "scope": scope, "kind": kind.as_str(),
                   "value": value, "outcome": result.get("outcome")}),
        );
        if let Err(e) = logged {
            eprintln!("warn: sandbox decision {id} not logged: {e:#}");
        }
        let stage = match effect {
            "dismissed" => Stage::Effect("dismissed", None),
            _ if agent.is_some() => Stage::Sent,
            e => Stage::Effect(e, None),
        };
        self.advance(id, stage);
        result["effect"] = json!(effect);
        result["lifecycle"] = self.lifecycle(id);
        self.outcome(&item, by, effect);
        Ok(result)
    }
}

#[cfg(test)]
mod tests;
