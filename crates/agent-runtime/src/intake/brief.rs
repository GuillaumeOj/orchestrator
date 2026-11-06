//! Plan with the Overwatch (P4.5 spec §3.5, api-contract §4–§5, D6): the
//! Overwatch's `propose_brief` makes a decision of kind `brief` (level 4,
//! like `rule_proposal`: never notifies, not in the badge), threaded under
//! the user's planning message; a new proposal in the same thread replaces
//! the open one (`revised`). Launch and Edit only open the prefilled dialog;
//! the card closes as `launched` when the user's submit starts it. The
//! Overwatch never starts anything (ADR-011).

use anyhow::{Result, bail};
use orch_store::{InboxItem, InboxStatus};
use serde_json::{Value, json};

use crate::inbox::Stage;
use crate::overwatch::tools::{Out, bad, require};
use crate::runtime::Runtime;
use crate::team::str_arg;

pub const TITLE_MAX: usize = 80;
pub const BRIEF_MAX: usize = 8000;
/// Below rule proposals in the Inbox order.
const PRIORITY_BRIEF: i64 = 5;

impl Runtime {
    fn open_briefs(&self) -> Vec<InboxItem> {
        self.store.read(|s| s.inbox.iter().filter(|i| i.kind == "brief" && i.open()).cloned().collect())
    }

    /// `propose_brief {project, title, slug?, brief, mode, team?, reply_to?}`.
    pub(crate) fn ow_propose_brief(&self, args: &Value) -> Out {
        let project = require(args, "project")?;
        let p = self.db.project(project)?.ok_or_else(|| bad(format!("no project {project:?}")))?;
        let title = str_arg(args, "title").map(str::trim).unwrap_or("");
        if title.is_empty() || title.chars().count() > TITLE_MAX {
            return Err(bad(format!("`title` is 1 to {TITLE_MAX} characters")));
        }
        let brief = str_arg(args, "brief").map(str::trim).unwrap_or("");
        if brief.is_empty() || brief.chars().count() > BRIEF_MAX {
            return Err(bad(format!("`brief` is 1 to {BRIEF_MAX} characters of markdown")));
        }
        let mode = match str_arg(args, "mode") {
            Some(m @ ("team" | "solo")) => m,
            _ => return Err(bad("`mode` is team or solo")),
        };
        let team = match (mode, str_arg(args, "team")) {
            ("team", Some(t)) => {
                self.db.team_in(Some(&p.id), t)?.ok_or_else(|| bad(format!("no team {t:?} in {project}")))?;
                Some(t.to_string())
            }
            ("team", None) => Some(
                self.intake_team(&p, None).ok_or_else(|| bad(format!("{project} has no team: propose mode solo")))?,
            ),
            _ => None,
        };
        // The slug: given (free, valid) or derived from the title.
        let slug = match str_arg(args, "slug").map(str::trim).filter(|s| !s.is_empty()) {
            Some(s) => {
                if !orch_team::valid_slug(s) || s == "solo" {
                    return Err(bad(format!("slug {s:?}: a-z, 0-9, - and _, at most 40")));
                }
                if let Some(f) = self.db.feature_by_slug(&p.id, s)?
                    && !f.archived
                {
                    return Err(bad(format!("slug {s:?} is taken by a feature of {project}")));
                }
                s.to_string()
            }
            None => self.intake_slug(&p.id, title),
        };
        // Threaded under the planning message (the one this turn answers).
        let reply_to = str_arg(args, "reply_to").map(str::to_string).or_else(|| self.overwatch_reply_target());
        // The thread of the message answered (its own id when it is a root).
        let thread = match reply_to.as_deref() {
            Some(r) => {
                let n = r.trim_start_matches("c-").parse::<u64>().ok();
                let e = n.and_then(|n| self.db.chat_get(n).ok().flatten());
                let e = e.ok_or_else(|| bad(format!("reply_to {r:?}: no such chat entry")))?;
                Some(e.thread.clone().unwrap_or_else(|| e.id()))
            }
            None => None,
        };
        let est = self.intake_est();
        let estimate = super::estimate::estimate(&est, &p.id, Some("M"), mode);
        let payload = json!({
            "project": p.id, "project_name": p.name, "title": title, "slug": slug, "brief": brief,
            "mode": mode, "team": team, "reply_to": reply_to, "thread": thread,
            "estimate": estimate, "notify": false,
        });
        let body = format!("Brief · {title}");
        // The open one of the same thread is replaced.
        let revised: Vec<u64> = match &thread {
            Some(t) => {
                self.open_briefs().into_iter().filter(|i| i.thread.as_deref() == Some(t)).map(|i| i.id).collect()
            }
            None => vec![],
        };
        let id = self.add_item_in(None, "brief", body, payload, PRIORITY_BRIEF, thread)?;
        for old in revised {
            self.brief_close(old, "revised", "revised", None, crate::overwatch::ID);
        }
        let decision = crate::decision::decision_id("brief", id);
        self.ow_audit(None, "propose_brief", Some(&decision), json!({"project": p.id, "mode": mode}));
        self.touch("inbox");
        Ok(json!({"ok": true, "decision": decision}))
    }

    /// Closes brief `n` as `word` by `by`, with `effect` (launched,
    /// dismissed, revised) and its detail (the slug or agent launched).
    pub(crate) fn brief_close(&self, n: u64, word: &str, effect: &'static str, detail: Option<String>, by: &str) {
        if let Some(i) = self.close_item(n, InboxStatus::Decided, word, by) {
            let _ = self.db.audit(by, None, "brief", Some(word), "ok", json!({"item": n}));
            self.advance(n, Stage::Effect(effect, detail));
            self.outcome(&i, by, effect);
            self.touch("inbox");
        }
    }

    /// Launch · Edit · Dismiss on `brief-<n>` (`inbox_act`). Launch and Edit
    /// open the dialog prefilled (`routeOpen {open: "launch"}`): nothing is
    /// started here.
    pub fn brief_act(&self, n: u64, action: &str) -> Result<Value> {
        let id = crate::decision::decision_id("brief", n);
        let item = self.find_brief(&id)?;
        if item.ended() && action == "dismiss" {
            self.close_item(n, InboxStatus::Dismissed, "dismiss", "user");
            return Ok(json!({"ok": true, "effect": "dismissed", "lifecycle": self.lifecycle(n)}));
        }
        if !item.open() {
            return Err(super::start::stale("brief closed", None));
        }
        match action {
            "launch" | "edit" => {
                let mode = item.payload["mode"].as_str().map(str::to_string);
                let prefill = self.intake_prefill(None, Some(&id), mode.as_deref())?;
                Ok(json!({
                    "ok": true, "open": "launch", "prefill": prefill, "intake": {"brief_decision": id},
                    "focus": if action == "edit" { json!("brief") } else { Value::Null },
                    "project": item.payload["project"], "lifecycle": self.lifecycle(n),
                }))
            }
            "dismiss" => {
                self.brief_close(n, "dismiss", "dismissed", None, "user");
                Ok(json!({"ok": true, "effect": "dismissed", "lifecycle": self.lifecycle(n)}))
            }
            other => bail!("{other} does not apply to a brief"),
        }
    }

    /// A removed project's open briefs go stale.
    pub(crate) fn briefs_stale_for(&self, project: &str) {
        for i in self.open_briefs().into_iter().filter(|i| i.payload["project"].as_str() == Some(project)) {
            self.advance(i.id, Stage::Stale("project removed"));
        }
        self.touch("inbox");
    }
}
