//! The day's briefing (spec §3.3): what the core knows, written to
//! `briefing.md` in the day folder at each fresh session, at most
//! [`super::BRIEFING_MAX`] characters (≈ 8 k tokens). The first turn says
//! to read it.

use super::Event;
use crate::runtime::Runtime;

/// Sections in order; a long one is cut, the rest by reference (`world`).
fn cap(sections: Vec<(String, String)>, max: usize) -> String {
    let mut out = String::new();
    for (title, body) in sections {
        let head = format!("## {title}\n\n");
        let room = max.saturating_sub(out.len() + head.len() + 64);
        if room < 200 {
            out.push_str(&format!("## {title}\n\n(left out: call world() or inspect)\n\n"));
            continue;
        }
        out.push_str(&head);
        if body.len() > room {
            let cut: String = body.chars().take(room).collect();
            let cut = cut.rsplit_once('\n').map_or(cut.as_str(), |(a, _)| a).to_string();
            out.push_str(&cut);
            out.push_str("\n… (cut: call world() or inspect for the rest)\n\n");
        } else {
            out.push_str(body.trim_end());
            out.push_str("\n\n");
        }
    }
    out.trim_end().to_string() + "\n"
}

impl Runtime {
    /// The briefing text for `day` (`pending`: the user's messages kept while
    /// it was down or off, marked "sent earlier").
    pub(crate) fn overwatch_briefing(&self, day: &str, now: u64, pending: &[Event]) -> String {
        let mut sections = vec![];
        // What it is, and the hard lists in words.
        let hard: Vec<String> = crate::policy::hard::describe()
            .into_iter()
            .map(|(what, kind)| {
                let k = match kind {
                    crate::policy::hard::HardKind::Deny => "always denied",
                    crate::policy::hard::HardKind::UserOnly => "the user only",
                };
                format!("- {what}: {k}")
            })
            .collect();
        sections.push((
            "The core's hard lists".to_string(),
            format!(
                "The core checks these on every path, before the policy and before you; you can annotate, never allow them.\n\n{}",
                hard.join("\n")
            ),
        ));
        if !pending.is_empty() {
            let lines: Vec<String> = pending
                .iter()
                .map(|e| {
                    let chat = e.chat.as_deref().map(|c| format!(" (chat:{c})")).unwrap_or_default();
                    format!("- sent earlier{chat}: {}", e.text.as_deref().unwrap_or(&e.summary).replace('\n', " "))
                })
                .collect();
            sections.push(("Messages from the user while you were away".into(), lines.join("\n")));
        }
        // The attention snapshot, as `orchd now` prints it.
        let snap = self.now_snapshot();
        sections.push(("Agents and runs now".into(), crate::now::render(&snap, now)));
        let decisions: Vec<String> = snap["decisions"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|d| matches!(d["lifecycle"]["state"].as_str(), Some("open" | "parked")))
            .map(|d| {
                let s = |k: &str| d[k].as_str().unwrap_or("").to_string();
                format!("- {} [{}] {} · {} · waits on {}", s("id"), s("kind"), s("title"), s("summary"), s("waits_on"))
            })
            .collect();
        sections
            .push(("Open decisions".into(), if decisions.is_empty() { "None.".into() } else { decisions.join("\n") }));
        sections.push(("The week".into(), self.overwatch_week_text()));
        sections.push(("Projects and running features".into(), self.overwatch_projects_text()));
        let files: Vec<String> = self
            .policy_files()
            .iter()
            .map(|f| {
                let s = |k: &str| f[k].as_str().unwrap_or("").to_string();
                let label =
                    if s("scope") == "global" { "global".to_string() } else { format!("project {}", s("project")) };
                let err =
                    f.get("error").filter(|e| !e.is_null()).map(|e| format!(" · broken: {e}")).unwrap_or_default();
                format!("- {label}: {} · {} rule(s){err}", s("path"), f["rules"].as_u64().unwrap_or(0))
            })
            .collect();
        sections.push(("Policy files today".into(), files.join("\n")));
        let title = format!(
            "# Overwatch briefing · {day}\n\nWritten by the core at the start of your session. Use world() and inspect(agent) for anything newer.\n\n"
        );
        title + &cap(sections, super::BRIEFING_MAX - 200)
    }

    fn overwatch_week_text(&self) -> String {
        let now = self.now.clock().0;
        let snap = self.with_governor(|g| {
            let t = governor::jiff::Timestamp::from_millisecond(now as i64).unwrap_or_default();
            (g.snapshot(t), g.plan().clone(), g.is_paused_all())
        });
        let (s, plan, paused) = snap;
        let mut out = format!("- used {:.0} % of the week; the plan says {:.0} % by now", s.used, s.plan);
        if paused {
            out.push_str("\n- Pause all is on");
        }
        for a in &plan.allocations {
            out.push_str(&format!(
                "\n- allocation {} ({}): weight {:.0}, tier {:?}",
                a.id.0, a.target, a.weight, a.tier
            ));
        }
        out
    }

    fn overwatch_projects_text(&self) -> String {
        let projects = self.db.projects(false).unwrap_or_default();
        let runs = self.db.runs(None, true).unwrap_or_default();
        let features = self.db.features(None).unwrap_or_default();
        let mut out = vec![];
        for p in &projects {
            out.push(format!("- {} ({}): {}", p.name, p.id, p.path.display()));
            for f in features.iter().filter(|f| f.project_id == p.id && !f.archived) {
                let Some(run) = runs.iter().find(|r| r.feature_id == f.id) else { continue };
                let docs = f
                    .integration_worktree
                    .as_ref()
                    .map(|w| w.join(".orchestrator/features").join(&f.slug))
                    .filter(|d| d.is_dir());
                let files: Vec<String> = docs
                    .as_ref()
                    .and_then(|d| std::fs::read_dir(d).ok())
                    .into_iter()
                    .flatten()
                    .flatten()
                    .filter(|e| e.path().extension().is_some_and(|x| x == "md"))
                    .map(|e| e.path().display().to_string())
                    .collect();
                out.push(format!(
                    "  - feature {} ({}) · run {} {:?}{}",
                    f.title,
                    f.slug,
                    run.id,
                    run.status,
                    if files.is_empty() { String::new() } else { format!("\n    docs: {}", files.join(", ")) }
                ));
            }
        }
        if out.is_empty() { "No registered project.".into() } else { out.join("\n") }
    }
}
