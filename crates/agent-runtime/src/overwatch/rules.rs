//! Learned rules (spec §3.7, FR-1308, D6). The core counts, per kind of call
//! — (project, role, tool, the decision's `always` pattern, answer) — the
//! decisions the user answered with Allow once or Deny. At the threshold (3)
//! it hands the Overwatch a `rule_candidate`; the proposal is a decision of
//! kind `rule_proposal` (level 4: blocks nobody, never notifies, not in the
//! badge). The Overwatch off, down, or silent for the turn: the core
//! proposes the rule itself. Add rule writes the policy file; Not now
//! resets the count, and a kind of call declined twice is never proposed
//! again; Edit rule opens the file with the rule appended, commented out.

use std::collections::BTreeMap;

use anyhow::{Result, anyhow, bail};
use orch_store::{InboxItem, InboxStatus};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::tools::{Out, ToolErr, err};
use super::{Event, ID, Kind};
use crate::inbox::Stage;
use crate::policy::{Effect, Rule, Scope};
use crate::runtime::Runtime;
use crate::team::{cut, str_arg};

/// The settings key the tallies are kept under.
const KEY: &str = "overwatch.rules";
/// Low in the Inbox's ladder: it blocks nobody.
const PRIORITY_PROPOSAL: i64 = 5;
/// After this many declines, a kind of call is never proposed again (D6).
pub const DECLINES_MAX: u32 = 2;

/// What is known of one kind of call.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Tally {
    /// User answers since the last decline.
    pub count: u32,
    /// The decisions (`inbox-<n>`) behind the count.
    pub from: Vec<String>,
    pub declined: u32,
    pub declined_at_ms: Option<u64>,
    /// The open proposal's item.
    pub open: Option<u64>,
    /// A candidate was handed to the Overwatch and nothing came of it yet.
    pub pending: bool,
    /// The rule the candidate suggests.
    pub suggestion: Option<Value>,
}

pub(crate) type Tallies = BTreeMap<String, Tally>;

/// `(project, role, tool, pattern, answer)` as one key.
fn key_of(project: &str, role: &str, tool: &str, pattern: &str, allow: bool) -> String {
    format!("{project}\u{1f}{role}\u{1f}{tool}\u{1f}{pattern}\u{1f}{}", if allow { "allow" } else { "deny" })
}

/// What a proposal is made of.
pub(crate) struct Spec {
    pub pattern: String,
    pub effect: Effect,
    pub project: Option<String>,
    pub role: Option<String>,
    pub from: Vec<String>,
    pub text: String,
    pub source: &'static str,
}

impl Runtime {
    fn tallies(&self) -> std::sync::MutexGuard<'_, Option<Tallies>> {
        let mut g = self.overwatch.rules.lock().unwrap_or_else(|e| e.into_inner());
        if g.is_none() {
            *g = Some(
                self.db.setting(KEY).ok().flatten().and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default(),
            );
        }
        g
    }

    /// Runs `f` on the tallies and keeps them.
    fn with_tallies<R>(&self, f: impl FnOnce(&mut Tallies) -> R) -> R {
        let mut g = self.tallies();
        let t = g.as_mut().expect("loaded");
        let r = f(t);
        let _ = self.db.set_setting(KEY, &json!(t));
        r
    }

    /// The key of the kind of call a permission item was.
    fn key_of_item(i: &InboxItem, allow: bool) -> Option<String> {
        let p = &i.payload;
        let pattern = p.get("always")?.as_str()?;
        let project = p.get("project_id")?.as_str()?;
        let role = p.get("role").and_then(Value::as_str).unwrap_or("solo");
        Some(key_of(project, role, p.get("tool")?.as_str()?, pattern, allow))
    }

    // ------------------------------------------------------------ counting

    /// A permission the user answered (`decide`): Allow once or Deny counts
    /// toward a rule (Always allow already makes one).
    pub(crate) fn rules_learn(&self, item: &InboxItem, allow: bool) {
        let Some(key) = Self::key_of_item(item, allow) else { return };
        let threshold = self.overwatch_settings().rule_threshold.max(2);
        let p = &item.payload;
        let id = crate::decision::decision_id("permission", item.id);
        let candidate = self.with_tallies(|t| {
            let e = t.entry(key.clone()).or_default();
            e.count += 1;
            e.from.push(id);
            if e.open.is_some() || e.pending || e.declined >= DECLINES_MAX || e.count < threshold {
                return None;
            }
            e.pending = true;
            let c = json!({
                "match": p["always"], "effect": if allow { "allow" } else { "deny" }, "scope": "project",
                "project": p["project_id"], "role": p.get("role").filter(|r| r.is_string()).cloned()
                    .unwrap_or_else(|| json!("solo")),
                "from": e.from.iter().rev().take(threshold as usize).rev().collect::<Vec<_>>(), "key": key,
            });
            e.suggestion = Some(c.clone());
            Some(c)
        });
        if let Some(c) = candidate {
            self.rules_candidate(c);
        }
    }

    /// Hands a candidate to the Overwatch, or proposes it ourselves when it
    /// can't take it.
    fn rules_candidate(&self, c: Value) {
        let key = c["key"].as_str().unwrap_or("").to_string();
        let st = self.overwatch_status().status;
        if matches!(st, super::Status::Off | super::Status::Down | super::Status::Paused) {
            self.rules_propose_core(&key);
            return;
        }
        let mut ev = Event::new(
            Kind::RuleCandidate,
            format!(
                "you answered the same call {} times: {} {} for {}",
                c["from"].as_array().map_or(0, Vec::len),
                c["effect"].as_str().unwrap_or(""),
                c["match"].as_str().unwrap_or(""),
                c["role"].as_str().unwrap_or("")
            ),
        );
        ev.decision = c["from"].as_array().and_then(|a| a.last()).and_then(Value::as_str).map(str::to_string);
        ev.candidate = Some(c);
        if self.overwatch_enqueue(ev).is_none() {
            self.rules_propose_core(&key);
        }
    }

    /// The turn that carried these events ended (or was abandoned): a
    /// candidate it made no proposal for is proposed by the core.
    pub(crate) fn rules_unanswered(&self, events: &[Event]) {
        for ev in events.iter().filter(|e| e.kind == Kind::RuleCandidate) {
            if let Some(key) = ev.candidate.as_ref().and_then(|c| c["key"].as_str()) {
                self.rules_propose_core(key);
            }
        }
    }

    /// The core proposes the suggested rule of `key`, when still wanted.
    fn rules_propose_core(&self, key: &str) {
        let spec = self.with_tallies(|t| {
            let e = t.get_mut(key)?;
            if !e.pending || e.open.is_some() {
                return None;
            }
            let c = e.suggestion.clone()?;
            Some(Spec {
                pattern: c["match"].as_str()?.to_string(),
                effect: Effect::parse(c["effect"].as_str()?)?,
                project: c["project"].as_str().map(str::to_string),
                role: c["role"].as_str().map(str::to_string),
                from: e.from.clone(),
                text: String::new(),
                source: "core",
            })
        });
        if let Some(spec) = spec
            && let Err(e) = self.rules_create_proposal(key, spec)
        {
            eprintln!("warn: rule proposal for {key:?} not made: {} ({})", e.message, e.code);
        }
    }

    // ------------------------------------------------------------ the proposal

    /// The policy file a rule of `project` / global would go to.
    fn proposal_scope(project: Option<&str>) -> Scope {
        project.map_or(Scope::Global, |p| Scope::Project(p.into()))
    }

    /// Creates the `rule_proposal` decision for `key`.
    pub(crate) fn rules_create_proposal(&self, key: &str, spec: Spec) -> Result<u64, ToolErr> {
        let scope = Self::proposal_scope(spec.project.as_deref());
        if let Some(p) = &spec.project {
            self.db.project(p).ok().flatten().ok_or_else(|| err("bad_request", format!("no project {p:?}")))?;
        }
        // Never a rule that would allow what the core's lists keep.
        let rule = Rule {
            id: "proposal".into(),
            pattern: spec.pattern.clone(),
            effect: spec.effect,
            role: spec.role.clone(),
            agent: None,
            from: None,
            added_ms: None,
            note: None,
        };
        if let Some((_, why)) = crate::policy::neutralised(std::slice::from_ref(&rule)).into_iter().next() {
            return Err(err("not_allowed", why));
        }
        let path = self.policy_path(&scope).map_err(|e| err("bad_request", format!("{e:#}")))?;
        let (threshold, declined, declined_at) = {
            let t = self.with_tallies(|t| t.get(key).cloned());
            (
                self.overwatch_settings().rule_threshold,
                t.as_ref().map_or(0, |t| t.declined),
                t.and_then(|t| t.declined_at_ms),
            )
        };
        // One open per kind of call.
        if self.with_tallies(|t| t.get(key).is_some_and(|e| e.open.is_some())) {
            return Err(err("duplicate", "a proposal for this kind of call is already open"));
        }
        let links: Vec<Value> = spec
            .from
            .iter()
            .rev()
            .take(threshold as usize)
            .rev()
            .map(|d| {
                let label = d
                    .strip_prefix("inbox-")
                    .and_then(|n| n.parse::<u64>().ok())
                    .and_then(|n| self.store.read(|s| s.inbox.iter().find(|i| i.id == n).map(|i| i.body.clone())))
                    .map(|b| cut(&b, 80))
                    .unwrap_or_else(|| d.clone());
                let chat = self.db.chat_by_decision(d).ok().flatten().map(|e| e.id());
                json!({"decision": d, "label": label, "chat": chat})
            })
            .collect();
        let who = spec.role.as_deref().map(|r| format!(" for {r}")).unwrap_or_default();
        let place = spec.project.as_deref().map(|p| format!(" in {p}")).unwrap_or_default();
        let did = if spec.effect == Effect::Deny { "denied" } else { "allowed" };
        let text = if spec.text.trim().is_empty() {
            format!(
                "You {did} this call {} times{who}{place}. Add a rule so it is {did} without asking?",
                spec.from.len().max(threshold as usize)
            )
        } else {
            spec.text.clone()
        };
        let payload = json!({
            "match": spec.pattern, "effect": spec.effect.as_str(),
            "scope": if spec.project.is_some() { "project" } else { "global" },
            "project": spec.project, "role": spec.role,
            "file": path.display().to_string(), "file_label": scope.label(),
            "text": cut(&text, 600), "links": links, "from": spec.from, "key": key, "source": spec.source,
            "count": spec.from.len(), "threshold": threshold, "declined_before": declined,
            "declined_at_ms": declined_at, "notify": false,
        });
        let body = format!("Rule · {} {}", spec.effect.as_str(), spec.pattern);
        let id = self
            .add_item(None, "rule_proposal", body, payload, PRIORITY_PROPOSAL)
            .map_err(|e| err("bad_request", format!("{e:#}")))?;
        self.with_tallies(|t| {
            let e = t.entry(key.to_string()).or_default();
            e.open = Some(id);
            e.pending = false;
        });
        self.ow_audit(None, "propose_rule", Some(&spec.pattern), json!({"item": id, "source": spec.source}));
        Ok(id)
    }

    /// `propose_rule {match, effect, scope, project?, role?, from[], why}`.
    pub(crate) fn ow_propose_rule(&self, args: &Value) -> Out {
        let pattern = str_arg(args, "match").ok_or_else(|| err("bad_request", "`match` is required"))?;
        let effect = str_arg(args, "effect")
            .and_then(Effect::parse)
            .ok_or_else(|| err("bad_request", "`effect` is allow, deny, overwatch or user"))?;
        let (project, scope) = match str_arg(args, "scope") {
            Some("project") => (
                Some(
                    str_arg(args, "project")
                        .ok_or_else(|| err("bad_request", "scope project needs `project`"))?
                        .to_string(),
                ),
                "project",
            ),
            Some("global") => (None, "global"),
            _ => return Err(err("bad_request", "`scope` is project or global")),
        };
        let _ = scope;
        let from: Vec<String> = args
            .get("from")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
            .unwrap_or_default();
        if from.is_empty() {
            return Err(err("bad_request", "`from` lists the decisions it generalises"));
        }
        let role = str_arg(args, "role").map(str::to_string);
        // The kind of call: from the first decision it comes from.
        let key = from
            .iter()
            .filter_map(|d| d.strip_prefix("inbox-")?.parse::<u64>().ok())
            .find_map(|n| {
                let i = self.store.read(|s| s.inbox.iter().find(|i| i.id == n).cloned())?;
                let allow = matches!(i.decision.as_deref(), Some("approve" | "always_allow"));
                Self::key_of_item(&i, allow)
            })
            .unwrap_or_else(|| {
                key_of(
                    project.as_deref().unwrap_or("-"),
                    role.as_deref().unwrap_or("-"),
                    "-",
                    pattern,
                    effect == Effect::Allow,
                )
            });
        let why = cut(str_arg(args, "why").unwrap_or(""), 400);
        let spec = Spec { pattern: pattern.into(), effect, project, role, from, text: why, source: ID };
        let id = self.rules_create_proposal(&key, spec)?;
        Ok(json!({"ok": true, "decision": crate::decision::decision_id("rule_proposal", id)}))
    }

    // ------------------------------------------------------------ the user's actions

    /// Add rule · Not now · Edit rule on `rule-<n>` (`inbox_act`).
    pub fn rule_proposal_act(&self, n: u64, action: &str) -> Result<Value> {
        let item = self.store.read(|s| s.inbox.iter().find(|i| i.id == n && i.kind == "rule_proposal").cloned());
        let item = item.ok_or_else(|| anyhow!("no rule proposal {n}"))?;
        if !item.open() {
            bail!("the proposal is already {:?}", item.status);
        }
        let p = &item.payload;
        let key = p["key"].as_str().unwrap_or("").to_string();
        let pattern = p["match"].as_str().unwrap_or("").to_string();
        let effect = Effect::parse(p["effect"].as_str().unwrap_or("")).ok_or_else(|| anyhow!("a broken proposal"))?;
        let project = p["project"].as_str().map(str::to_string);
        let role = p["role"].as_str().map(str::to_string);
        let scope = Self::proposal_scope(project.as_deref());
        match action {
            "add" => {
                let from = Some(format!("proposal:{}", crate::decision::decision_id("rule_proposal", n)));
                let r = self.add_rule_to(&scope, role, None, &pattern, effect, from)?;
                let (rid, file) = (r["rule"]["id"].clone(), scope.label());
                self.set_payload(n, &[("added_rule_id", rid), ("added_file", json!(file))]);
                self.close_proposal(n, "add", "added", Some(format!("added to {}", file_name(p))));
                self.with_tallies(|t| t.remove(&key));
                Ok(json!({"ok": true, "effect": "added", "lifecycle": self.lifecycle(n), "result": r}))
            }
            "not_now" => {
                self.close_proposal(n, "not_now", "not_now", None);
                let now = orch_protocol::now_ms();
                self.with_tallies(|t| {
                    let e = t.entry(key).or_default();
                    e.declined += 1;
                    e.declined_at_ms = Some(now);
                    e.count = 0;
                    e.from.clear();
                    e.open = None;
                    e.pending = false;
                });
                Ok(json!({"ok": true, "effect": "not_now", "lifecycle": self.lifecycle(n)}))
            }
            "edit" => {
                let path = self.policy_path(&scope)?;
                let rule = Rule {
                    id: format!("proposal-{n}"),
                    pattern,
                    effect,
                    role,
                    agent: None,
                    from: Some(format!("proposal:{}", crate::decision::decision_id("rule_proposal", n))),
                    added_ms: None,
                    note: None,
                };
                let _w = self.policy.write_lock();
                crate::policy::file::append_commented_rule(&path, &rule)?;
                drop(_w);
                self.set_payload(n, &[("edited_at_ms", json!(orch_protocol::now_ms()))]);
                self.touch("inbox");
                Ok(json!({"ok": true, "open": "editor", "path": path.display().to_string(),
                          "scope": scope.label(), "lifecycle": self.lifecycle(n)}))
            }
            other => bail!("{other} does not apply to a rule proposal"),
        }
    }

    /// Closes a proposal as the user's decision `word`, with `effect`.
    fn close_proposal(&self, n: u64, word: &str, effect: &'static str, detail: Option<String>) {
        if let Some(i) = self.close_item(n, InboxStatus::Decided, word, "user") {
            let _ = self.db.audit("user", None, "rule_proposal", Some(word), "ok", json!({"item": n}));
            self.advance(n, Stage::Effect(effect, detail));
            self.outcome(&i, "user", effect);
        }
    }

    /// A policy file changed: a proposal the user edited by hand, whose rule
    /// is now active in its file, closes as added ("by hand", F7).
    pub(crate) fn rules_check_edited(&self) {
        let open: Vec<InboxItem> = self.store.read(|s| {
            s.inbox
                .iter()
                .filter(|i| i.kind == "rule_proposal" && i.open() && i.payload.get("edited_at_ms").is_some())
                .cloned()
                .collect()
        });
        for i in open {
            let p = &i.payload;
            let scope = Self::proposal_scope(p["project"].as_str());
            let Some(f) = self.policy.file(&scope) else { continue };
            let role = p["role"].as_str();
            let found = f.doc.rules.iter().find(|r| {
                r.pattern == p["match"].as_str().unwrap_or("")
                    && r.effect.as_str() == p["effect"].as_str().unwrap_or("")
                    && r.role.as_deref() == role
            });
            if let Some(r) = found {
                self.set_payload(i.id, &[("added_rule_id", json!(r.id)), ("added_file", json!(scope.label()))]);
                self.close_proposal(i.id, "add", "added", Some("by hand".into()));
                if let Some(k) = p["key"].as_str() {
                    self.with_tallies(|t| t.remove(k));
                }
            }
        }
    }
}

/// The file's short name for "added to <file>": the project's `policy.toml`
/// path's last two parts, else the global one.
fn file_name(p: &Value) -> String {
    let path = p["file"].as_str().unwrap_or("policy.toml");
    let parts: Vec<&str> = path.rsplit('/').take(2).collect();
    parts.into_iter().rev().collect::<Vec<_>>().join("/")
}
