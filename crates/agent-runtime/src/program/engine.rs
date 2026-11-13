//! The program engine (contract §4 "Engine"): every tick it reads each
//! feature's real state from its run or session, starts the planned
//! features whose (non-skipped) dependencies merged, within `max_parallel`,
//! and closes the program when everything merged. A feature starts exactly
//! as a user's start does (`launch` / `start_solo`), with **its** slug,
//! title, edited brief, mode, team and base; the Governor still admits it and
//! the spec and PR gates are unchanged. Idempotent: all state is in the
//! program's row.

use std::collections::HashSet;

use orch_protocol::AgentSpec;
use orch_team::RunState;
use serde_json::{Value, json};

use super::{Feature, Program};
use crate::intake::IntakeLink;
use crate::runtime::Runtime;
use crate::state::AgentState;
use crate::team::LaunchSpec;

/// What a feature's run or session says now.
struct Live {
    state: &'static str,
    phase: Option<String>,
    pr: Option<Value>,
    merged_at_ms: Option<u64>,
}

fn phase_of(s: RunState) -> &'static str {
    match s {
        RunState::Draft | RunState::Spec | RunState::SpecGate => "spec",
        RunState::Planning => "planning",
        RunState::Integrating => "integrating",
        RunState::PrGate | RunState::PrOpen => "pr",
        _ => "executing",
    }
}

impl Runtime {
    /// The features' live state, from their runs and sessions; `None` before they started.
    fn feature_live(&self, f: &Feature) -> Option<Live> {
        if let Some(id) = &f.run {
            let run = self.db.run(id).ok().flatten()?;
            return Some(match run.status {
                RunState::Done => Live { state: "merged", phase: None, pr: None, merged_at_ms: run.ended_at_ms },
                RunState::Failed => Live { state: "failed", phase: None, pr: None, merged_at_ms: None },
                RunState::PrOpen | RunState::PrGate if run.pr_url.is_some() => Live {
                    state: "pr",
                    phase: Some("pr".into()),
                    pr: Some(json!({"number": run.pr_number, "url": run.pr_url})),
                    merged_at_ms: None,
                },
                s => {
                    let s = if s.held() { run.resume_status.unwrap_or(s) } else { s };
                    // Held by the Governor: the lead has not been admitted yet.
                    let queued = self.store.read(|st| {
                        st.agents.values().any(|a| {
                            a.team.as_ref().is_some_and(|t| t.run_id == *id)
                                && crate::runtime::is_lead(a)
                                && matches!(a.tracker.state, AgentState::Queued | AgentState::Created)
                        })
                    });
                    Live {
                        state: if queued { "queued" } else { "running" },
                        phase: Some(phase_of(s).into()),
                        pr: None,
                        merged_at_ms: None,
                    }
                }
            });
        }
        if let Some(id) = &f.agent {
            let state = self.store.read(|s| s.agents.get(id).map(|a| a.tracker.state));
            let done = self.db.events(0, Some(id), Some("turn.done"), 1).map(|e| !e.is_empty()).unwrap_or(false);
            return Some(match state {
                // A solo session is done when it ended after a finished turn.
                Some(AgentState::Archived | AgentState::Killed) | None if done => {
                    Live { state: "merged", phase: None, pr: None, merged_at_ms: Some(orch_protocol::now_ms()) }
                }
                Some(AgentState::Archived | AgentState::Killed) | None => {
                    Live { state: "failed", phase: None, pr: None, merged_at_ms: None }
                }
                Some(AgentState::Queued | AgentState::Created) => {
                    Live { state: "queued", phase: None, pr: None, merged_at_ms: None }
                }
                Some(_) => Live { state: "running", phase: Some("executing".into()), pr: None, merged_at_ms: None },
            });
        }
        None
    }

    /// One pass over every program in progress (also kicked by run events and refreshes).
    /// A pass already running makes this one a no-op.
    pub fn programs_tick(&self) {
        let Ok(_guard) = self.intake.prog_lock.try_lock() else { return };
        for v in self.db.programs(None).unwrap_or_default() {
            let Ok(mut p) = serde_json::from_value::<Program>(v) else { continue };
            if !matches!(p.status.as_str(), "running" | "paused") {
                continue;
            }
            let before = p.clone();
            self.program_step(&mut p);
            if p != before
                && let Err(e) = self.program_save(&p)
            {
                eprintln!("warn: program {} not saved: {e:#}", p.id);
            }
        }
    }

    fn program_step(&self, p: &mut Program) {
        let now = orch_protocol::now_ms();
        // 1. What each started feature does now.
        for f in p.features.iter_mut().filter(|f| !f.skip) {
            if let Some(l) = self.feature_live(f) {
                (f.state, f.phase, f.pr) = (l.state.into(), l.phase, l.pr);
                if l.merged_at_ms.is_some() {
                    f.merged_at_ms = f.merged_at_ms.or(l.merged_at_ms);
                }
                if f.state == "merged" && !p.notified.contains(&format!("merged:{}", f.key)) {
                    p.notified.push(format!("merged:{}", f.key));
                    self.event("-", "program.feature_merged", json!({"id": p.id, "feature": f.slug}));
                }
            }
        }
        // 2. Not started: waiting for a dependency, or planned.
        let merged: HashSet<String> =
            p.features.iter().filter(|f| f.state == "merged").map(|f| f.key.clone()).collect();
        for f in p.features.iter_mut().filter(|f| !f.skip && f.run.is_none() && f.agent.is_none()) {
            if matches!(f.state.as_str(), "planned" | "waiting") {
                f.state =
                    if f.depends_on.iter().all(|d| merged.contains(d)) { "planned".into() } else { "waiting".into() };
            }
        }
        // 3. Start what may, in order, while there is room (a paused program starts nothing).
        if p.status == "running" {
            let mut active =
                p.features.iter().filter(|f| matches!(f.state.as_str(), "queued" | "running" | "pr")).count() as u32;
            for i in 0..p.features.len() {
                if active >= p.max_parallel {
                    break;
                }
                if p.features[i].state == "planned" && !p.features[i].skip && self.start_feature(p, i) {
                    active += 1;
                }
            }
        }
        // 4. Failures are decisions; the program goes on (dependents stay waiting).
        let failed: Vec<(String, String, Option<String>)> = p
            .features
            .iter()
            .filter(|f| f.state == "failed" && !p.notified.contains(&format!("failed:{}", f.key)))
            .map(|f| (f.key.clone(), f.slug.clone(), f.error.clone()))
            .collect();
        for (key, slug, why) in failed {
            p.notified.push(format!("failed:{key}"));
            self.program_decision(
                p,
                "program_feature_failed",
                &format!("{slug} failed in {}", p.title),
                &why.unwrap_or_else(|| "The run ended without finishing.".into()),
                json!({"feature": key, "slug": slug}),
                &format!("failed:{key}"),
            );
        }
        // 5. Done: every feature that is not skipped merged.
        let live: Vec<&Feature> = p.features.iter().filter(|f| !f.skip).collect();
        if p.status == "running" && !live.is_empty() && live.iter().all(|f| f.state == "merged")
            || p.status == "running" && live.is_empty()
        {
            p.status = "done".into();
            p.done_at_ms = Some(now);
            let merged = p.features.iter().filter(|f| f.state == "merged").count();
            let skipped = p.features.iter().filter(|f| f.skip).count();
            if !p.notified.contains(&"done".to_string()) {
                p.notified.push("done".into());
                let (lo, hi) =
                    (p.estimate["low_pp"].as_f64().unwrap_or(0.0), p.estimate["high_pp"].as_f64().unwrap_or(0.0));
                self.program_decision(
                    p,
                    "program_done",
                    &format!("{} is done", p.title),
                    &format!("{merged} merged, {skipped} skipped · {lo:.0}-{hi:.0} pp planned."),
                    json!({}),
                    "done",
                );
            }
            self.event(
                "-",
                "program.done",
                json!({"id": p.id, "project": p.project, "title": p.title, "merged": merged}),
            );
        }
    }

    /// Starts feature `i` as the user would, with the values the user approved. False when it failed to start (it is then `failed`).
    fn start_feature(&self, p: &mut Program, i: usize) -> bool {
        let f = p.features[i].clone();
        let Some(first) = f.items.first() else { return false };
        let link = IntakeLink { source: Some(first.clone()), ..Default::default() };
        #[cfg(test)]
        let hooked = crate::intake::lock(&self.intake.test_starter).as_ref().map(|h| h(self, &f));
        #[cfg(not(test))]
        let hooked: Option<anyhow::Result<(&str, String)>> = None;
        let started: anyhow::Result<(&str, String)> = if let Some(h) = hooked {
            h
        } else if f.mode == "team" {
            match &f.team {
                Some(team) => self
                    .launch(&LaunchSpec {
                        project_id: p.project.clone(),
                        team_id: team.clone(),
                        slug: f.slug.clone(),
                        title: f.title.clone(),
                        brief: f.brief.clone(),
                        base: Some(f.base.clone()).filter(|b| !b.is_empty()),
                        off_hours: false,
                        intake: Some(link),
                    })
                    .map(|run| ("run", run.id)),
                None => Err(anyhow::anyhow!("no team for {}", f.slug)),
            }
        } else {
            self.program_start_solo(p, &f, &link).map(|id| ("agent", id))
        };
        match started {
            Ok((kind, reference)) => {
                {
                    let feat = &mut p.features[i];
                    if kind == "run" {
                        feat.run = Some(reference.clone());
                    } else {
                        feat.agent = Some(reference.clone());
                    }
                    feat.state = "running".into();
                    feat.error = None;
                }
                // Every item of the feature is linked to the run (app state and history on all of them).
                let by = json!({"id": p.id, "title": p.title});
                for id in f.items.iter().skip(1) {
                    if let Ok(r) = self.node_row(&p.project, id) {
                        let linked = self.db.intake_mark_started(r.id, kind, &reference).unwrap_or(false)
                            || self.db.intake_restart(r.id, kind, &reference).unwrap_or(false);
                        if linked {
                            let e = orch_store::WorkEvent {
                                at_ms: orch_protocol::now_ms(),
                                what: "started".into(),
                                feature: Some(f.slug.clone()),
                                program: Some(by.clone()),
                                detail: None,
                            };
                            let _ = self.db.work_note(&r.project_id, &r.node_id, &e);
                        }
                    }
                }
                self.event("-", "program.feature_started", json!({"id": p.id, "feature": f.slug}));
                true
            }
            Err(e) => {
                let feat = &mut p.features[i];
                feat.state = "failed".into();
                feat.error = Some(format!("{e:#}"));
                false
            }
        }
    }

    /// A solo feature: a session named for it, with the brief as its first message.
    fn program_start_solo(&self, p: &Program, f: &Feature, link: &IntakeLink) -> anyhow::Result<String> {
        let project = self.db.project(&p.project)?.ok_or_else(|| anyhow::anyhow!("no project {}", p.project))?;
        let id = self.intake_agent_name(&f.slug);
        let spec = AgentSpec {
            agent_id: id.clone(),
            repo: project.path.display().to_string(),
            slug: Some(id.clone()),
            base: Some(f.base.clone()).filter(|b| !b.is_empty()),
            first_message: Some(format!("Brief from the user:\n\n{}", f.brief)),
            allocation: Some(project.id.clone()),
            ..Default::default()
        };
        self.start_solo(&spec, &p.project, Some(link))?;
        Ok(id)
    }

    /// Posts a program decision once (`key` identifies it among the program's). Its card is in `decision.rs`.
    pub(crate) fn program_decision(&self, p: &Program, kind: &str, title: &str, text: &str, extra: Value, key: &str) {
        let mut payload = json!({"kind": kind, "program": p.id, "program_title": p.title, "project": p.project,
                                 "title": title, "text": text, "key": key});
        if let (Some(o), Some(e)) = (payload.as_object_mut(), extra.as_object()) {
            o.extend(e.clone());
        }
        let level = if matches!(kind, "program_dependency_added" | "program_done") { 4 } else { 3 };
        if let Err(e) = self.add_item_in(None, "program", format!("{title}\n{text}"), payload, 10 - level, None) {
            eprintln!("warn: program decision {kind} not posted: {e:#}");
        }
    }

    /// Closes the open decisions of program `p` whose key `pick` selects (`new:#19`, `failed:pf-2`…).
    pub(crate) fn program_close_decisions(&self, p: &Program, pick: impl Fn(&str) -> bool) {
        self.close_items_where(
            |i| {
                i.kind == "program"
                    && i.payload["program"].as_str() == Some(p.id.as_str())
                    && pick(i.payload["key"].as_str().unwrap_or(""))
            },
            "resolved",
        );
    }
}
