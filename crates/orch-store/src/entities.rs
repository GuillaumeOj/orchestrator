//! Typed access to the §7 entities that are not runtime-owned: projects,
//! permission rules, usage samples, the Governor's tables, terminals, settings.

use std::path::{Path, PathBuf};

use anyhow::Result;
use governor::jiff::Timestamp;
use governor::{Calibration, ForecastSnapshot, GovernorAction, PacingPolicy, QueueEntry, WeeklyPlan};
use rusqlite::{OptionalExtension, params};
use serde_json::Value;

use crate::db::Store;
use crate::model::*;
use crate::util::{from_js, js, json_row, opt_u64, p, tag};

fn ms(t: Timestamp) -> i64 {
    t.as_millisecond()
}

const PROJECT_COLS: &str = "id, name, path, default_branch, remotes, worktree_root, task_backend, track_orchestrator_dir, settings, created_at_ms, last_active_at_ms, last_fetch_at_ms, last_fetch_error, archived";

fn row_project(r: &rusqlite::Row) -> rusqlite::Result<Project> {
    Ok(Project {
        id: r.get(0)?,
        name: r.get(1)?,
        path: PathBuf::from(r.get::<_, String>(2)?),
        default_branch: r.get(3)?,
        remotes: from_js(&r.get::<_, String>(4)?)?,
        worktree_root: PathBuf::from(r.get::<_, String>(5)?),
        task_backend: from_js(&r.get::<_, String>(6)?)?,
        track_orchestrator_dir: r.get(7)?,
        settings: from_js(&r.get::<_, String>(8)?)?,
        created_at_ms: r.get::<_, i64>(9)? as u64,
        last_active_at_ms: r.get::<_, i64>(10)? as u64,
        last_fetch_at_ms: opt_u64(r, 11)?,
        last_fetch_error: r.get(12)?,
        archived: r.get(13)?,
    })
}

const RULE_COLS: &str = "id, scope, scope_id, pattern, effect, source_inbox_item_id, created_at_ms, revoked_at_ms";

fn row_rule(r: &rusqlite::Row) -> rusqlite::Result<PermissionRule> {
    let bad = |s: String| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, s.into());
    let scope: String = r.get(1)?;
    let effect: String = r.get(4)?;
    Ok(PermissionRule {
        id: r.get::<_, i64>(0)? as u64,
        scope: RuleScope::parse(&scope).ok_or_else(|| bad(scope))?,
        scope_id: r.get(2)?,
        pattern: r.get(3)?,
        effect: RuleEffect::parse(&effect).ok_or_else(|| bad(effect))?,
        source_inbox_item_id: opt_u64(r, 5)?,
        created_at_ms: r.get::<_, i64>(6)? as u64,
        revoked_at_ms: opt_u64(r, 7)?,
    })
}

const USAGE_COLS: &str = "id, ts_ms, agent_id, turn_id, model, tokens_in, tokens_out, cache_read, cache_write, cost_estimate, cost_delta, five_hour_pct, five_hour_resets_at, weekly_pct, weekly_resets_at, model_weekly_pct, provenance, source, class, allocation";

fn row_usage(r: &rusqlite::Row) -> rusqlite::Result<UsageRecord> {
    let u = |i: usize| opt_u64(r, i);
    let source: String = r.get(17)?;
    Ok(UsageRecord {
        id: r.get::<_, i64>(0)? as u64,
        ts_ms: r.get::<_, i64>(1)? as u64,
        agent_id: r.get(2)?,
        turn_id: r.get(3)?,
        model: r.get(4)?,
        tokens_in: u(5)?,
        tokens_out: u(6)?,
        cache_read: u(7)?,
        cache_write: u(8)?,
        cost_estimate: r.get(9)?,
        cost_delta: r.get(10)?,
        five_hour_pct: r.get(11)?,
        five_hour_resets_at: r.get(12)?,
        weekly_pct: r.get(13)?,
        weekly_resets_at: r.get(14)?,
        model_weekly_pct: r.get(15)?,
        provenance: r.get(16)?,
        source: crate::untag(&source),
        class: r.get(18)?,
        allocation: r.get(19)?,
    })
}

impl Store {
    // ------------------------------------------------------------ projects

    pub fn upsert_project(&self, pr: &Project) -> Result<()> {
        let pr = pr.clone();
        self.write(move |tx| {
            tx.execute(
                &format!(
                    "INSERT INTO projects({PROJECT_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)
                     ON CONFLICT(id) DO UPDATE SET name=excluded.name, path=excluded.path,
                       default_branch=excluded.default_branch, remotes=excluded.remotes,
                       worktree_root=excluded.worktree_root, task_backend=excluded.task_backend,
                       track_orchestrator_dir=excluded.track_orchestrator_dir, settings=excluded.settings,
                       last_active_at_ms=excluded.last_active_at_ms, last_fetch_at_ms=excluded.last_fetch_at_ms,
                       last_fetch_error=excluded.last_fetch_error, archived=excluded.archived"
                ),
                params![
                    pr.id,
                    pr.name,
                    p(&pr.path),
                    pr.default_branch,
                    js(&pr.remotes),
                    p(&pr.worktree_root),
                    js(&pr.task_backend),
                    pr.track_orchestrator_dir,
                    js(&pr.settings),
                    pr.created_at_ms as i64,
                    pr.last_active_at_ms as i64,
                    pr.last_fetch_at_ms.map(|v| v as i64),
                    pr.last_fetch_error,
                    pr.archived
                ],
            )?;
            Ok(())
        })
    }

    pub fn project(&self, id: &str) -> Result<Option<Project>> {
        let id = id.to_string();
        self.read(move |c| {
            Ok(c.query_row(&format!("SELECT {PROJECT_COLS} FROM projects WHERE id=?1"), [id], row_project)
                .optional()?)
        })
    }

    pub fn project_by_path(&self, path: &Path) -> Result<Option<Project>> {
        let key = p(path);
        self.read(move |c| {
            Ok(c.query_row(&format!("SELECT {PROJECT_COLS} FROM projects WHERE path=?1"), [key], row_project)
                .optional()?)
        })
    }

    /// Every project, archived ones included when `with_archived`.
    pub fn projects(&self, with_archived: bool) -> Result<Vec<Project>> {
        self.read(move |c| {
            let mut st =
                c.prepare(&format!("SELECT {PROJECT_COLS} FROM projects WHERE archived=0 OR ?1 ORDER BY name, id"))?;
            Ok(st.query_map([with_archived], row_project)?.collect::<rusqlite::Result<_>>()?)
        })
    }

    pub fn touch_project(&self, id: &str, at_ms: u64) -> Result<()> {
        let id = id.to_string();
        self.write(move |tx| {
            tx.execute(
                "UPDATE projects SET last_active_at_ms=max(last_active_at_ms, ?2) WHERE id=?1",
                params![id, at_ms as i64],
            )?;
            Ok(())
        })
    }

    pub fn record_fetch(&self, id: &str, at_ms: u64, error: Option<String>) -> Result<()> {
        let id = id.to_string();
        self.write(move |tx| {
            tx.execute(
                "UPDATE projects SET last_fetch_at_ms=?2, last_fetch_error=?3 WHERE id=?1",
                params![id, at_ms as i64, error],
            )?;
            Ok(())
        })
    }

    /// J11 "delete": the project row, its worktree rows, its rules and its
    /// transcript index. Events, usage and the audit log are kept (history).
    pub fn delete_project(&self, id: &str) -> Result<()> {
        let id = id.to_string();
        self.write(move |tx| {
            let path: Option<String> =
                tx.query_row("SELECT path FROM projects WHERE id=?1", [&id], |r| r.get(0)).optional()?;
            let Some(path) = path else { return Ok(()) };
            tx.execute("DELETE FROM worktrees WHERE repo=?1", [&path])?;
            tx.execute("DELETE FROM permission_rules WHERE scope='project' AND scope_id=?1", [&id])?;
            tx.execute("DELETE FROM transcript_fts WHERE project_id=?1", [&id])?;
            tx.execute("DELETE FROM transcript_files WHERE project_id=?1", [&id])?;
            tx.execute("DELETE FROM terminals WHERE project_id=?1", [&id])?;
            tx.execute("DELETE FROM projects WHERE id=?1", [&id])?;
            Ok(())
        })
    }

    // ------------------------------------------------------------ permission rules

    pub fn add_rule(
        &self,
        scope: RuleScope,
        scope_id: &str,
        pattern: &str,
        effect: RuleEffect,
        source_inbox_item_id: Option<u64>,
    ) -> Result<PermissionRule> {
        let (scope_id, pattern) = (scope_id.to_string(), pattern.to_string());
        let now = crate::now_ms();
        self.write(move |tx| {
            tx.execute(
                "INSERT INTO permission_rules(scope, scope_id, pattern, effect, source_inbox_item_id, created_at_ms)
                 VALUES (?1,?2,?3,?4,?5,?6)",
                params![
                    scope.as_str(),
                    scope_id,
                    pattern,
                    effect.as_str(),
                    source_inbox_item_id.map(|v| v as i64),
                    now as i64
                ],
            )?;
            let id = tx.last_insert_rowid() as u64;
            Ok(PermissionRule {
                id,
                scope,
                scope_id,
                pattern,
                effect,
                source_inbox_item_id,
                created_at_ms: now,
                revoked_at_ms: None,
            })
        })
    }

    /// Revokes (keeps the row for the audit trail). False if unknown or already revoked.
    pub fn revoke_rule(&self, id: u64) -> Result<bool> {
        let now = crate::now_ms();
        self.write(move |tx| {
            Ok(tx.execute(
                "UPDATE permission_rules SET revoked_at_ms=?2 WHERE id=?1 AND revoked_at_ms IS NULL",
                params![id as i64, now as i64],
            )? == 1)
        })
    }

    /// Active rules (not revoked), oldest first.
    pub fn rules(&self) -> Result<Vec<PermissionRule>> {
        self.read(|c| {
            let mut st = c.prepare(&format!(
                "SELECT {RULE_COLS} FROM permission_rules WHERE revoked_at_ms IS NULL ORDER BY id"
            ))?;
            Ok(st.query_map([], row_rule)?.collect::<rusqlite::Result<_>>()?)
        })
    }

    // ------------------------------------------------------------ usage samples

    /// Appends a sample without waiting (statusline/OTLP paths must not block).
    pub fn add_usage(&self, u: UsageRecord) {
        self.write_detached(move |tx| {
            let o = |v: Option<u64>| v.map(|x| x as i64);
            tx.execute(
                &format!(
                    "INSERT INTO usage_samples({}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19)",
                    &USAGE_COLS[4..]
                ),
                params![
                    u.ts_ms as i64,
                    u.agent_id,
                    u.turn_id,
                    u.model,
                    o(u.tokens_in),
                    o(u.tokens_out),
                    o(u.cache_read),
                    o(u.cache_write),
                    u.cost_estimate,
                    u.cost_delta,
                    u.five_hour_pct,
                    u.five_hour_resets_at,
                    u.weekly_pct,
                    u.weekly_resets_at,
                    u.model_weekly_pct,
                    u.provenance,
                    u.source.map(|s| s.as_str()).unwrap_or("statusline"),
                    u.class,
                    u.allocation
                ],
            )?;
            Ok(())
        });
    }

    /// Samples with `ts >= since_ms`, oldest first, optionally of one source.
    pub fn usage_since(&self, since_ms: u64, source: Option<UsageSource>) -> Result<Vec<UsageRecord>> {
        self.read(move |c| {
            let mut st = c.prepare(&format!(
                "SELECT {USAGE_COLS} FROM usage_samples WHERE ts_ms >= ?1 AND (?2 IS NULL OR source = ?2) ORDER BY ts_ms, id"
            ))?;
            Ok(st
                .query_map(params![since_ms as i64, source.map(|s| s.as_str())], row_usage)?
                .collect::<rusqlite::Result<_>>()?)
        })
    }

    /// Token and cost totals per agent since `since_ms` (OTLP rows).
    pub fn usage_totals(&self, since_ms: u64) -> Result<Vec<UsageTotal>> {
        self.read(move |c| {
            let mut st = c.prepare(
                "SELECT agent_id, coalesce(sum(tokens_in),0), coalesce(sum(tokens_out),0), coalesce(sum(cache_read),0),
                        coalesce(sum(cache_write),0), coalesce(sum(cost_estimate),0.0)
                 FROM usage_samples WHERE source='otel' AND ts_ms >= ?1 GROUP BY agent_id ORDER BY agent_id",
            )?;
            Ok(st
                .query_map([since_ms as i64], |r| {
                    Ok(UsageTotal {
                        agent_id: r.get(0)?,
                        tokens_in: r.get::<_, i64>(1)? as u64,
                        tokens_out: r.get::<_, i64>(2)? as u64,
                        cache_read: r.get::<_, i64>(3)? as u64,
                        cache_write: r.get::<_, i64>(4)? as u64,
                        cost: r.get(5)?,
                    })
                })?
                .collect::<rusqlite::Result<_>>()?)
        })
    }

    // ------------------------------------------------------------ weekly plan, policy

    /// Saves a plan and its allocations (replacing the allocation set).
    pub fn save_plan(&self, plan: &WeeklyPlan, status: PlanStatus) -> Result<()> {
        let plan = plan.clone();
        let now = crate::now_ms();
        self.write(move |tx| {
            tx.execute(
                "INSERT INTO weekly_plans(id, week_start_ms, reset_at_ms, status, shadow_mode, reserve_pct, data, updated_at_ms)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8)
                 ON CONFLICT(id) DO UPDATE SET week_start_ms=excluded.week_start_ms, reset_at_ms=excluded.reset_at_ms,
                   status=excluded.status, shadow_mode=excluded.shadow_mode, reserve_pct=excluded.reserve_pct,
                   data=excluded.data, updated_at_ms=excluded.updated_at_ms",
                params![
                    plan.id,
                    ms(plan.week_start),
                    ms(plan.reset_at),
                    status.as_str(),
                    plan.shadow_mode,
                    plan.reserve_pct,
                    js(&plan),
                    now as i64
                ],
            )?;
            if status == PlanStatus::Active {
                tx.execute("UPDATE weekly_plans SET status='closed' WHERE id<>?1 AND status='active'", [&plan.id])?;
            }
            tx.execute("DELETE FROM budget_allocations WHERE weekly_plan_id=?1", [&plan.id])?;
            for a in &plan.allocations {
                tx.execute(
                    "INSERT INTO budget_allocations(weekly_plan_id, id, target, weight, tier, hard_cap_pct) VALUES (?1,?2,?3,?4,?5,?6)",
                    params![plan.id, a.id.to_string(), a.target, a.weight, tag(&a.tier), a.hard_cap_pct],
                )?;
            }
            Ok(())
        })
    }

    /// The active plan, if any.
    pub fn active_plan(&self) -> Result<Option<WeeklyPlan>> {
        self.read(|c| {
            json_row(
                c,
                "SELECT data FROM weekly_plans WHERE status='active' ORDER BY reset_at_ms DESC LIMIT 1",
                [],
                "weekly plan",
            )
        })
    }

    /// The most recent plan that ended before `before` (Week planner prefill, J8).
    pub fn previous_plan(&self, before: Timestamp) -> Result<Option<WeeklyPlan>> {
        self.read(move |c| {
            json_row(
                c,
                "SELECT data FROM weekly_plans WHERE reset_at_ms <= ?1 ORDER BY reset_at_ms DESC LIMIT 1",
                [ms(before)],
                "weekly plan",
            )
        })
    }

    pub fn save_policy(&self, id: &str, policy: &PacingPolicy) -> Result<()> {
        let (id, data) = (id.to_string(), js(policy));
        let now = crate::now_ms();
        self.write(move |tx| {
            tx.execute(
                "INSERT INTO pacing_policies(id, data, updated_at_ms) VALUES (?1,?2,?3)
                 ON CONFLICT(id) DO UPDATE SET data=excluded.data, updated_at_ms=excluded.updated_at_ms",
                params![id, data, now as i64],
            )?;
            Ok(())
        })
    }

    pub fn policy(&self, id: &str) -> Result<Option<PacingPolicy>> {
        let id = id.to_string();
        self.read(move |c| json_row(c, "SELECT data FROM pacing_policies WHERE id=?1", [id], "pacing policy"))
    }

    // ------------------------------------------------------------ governor log

    /// Replaces the stored queue with the Governor's current one (for views;
    /// the Governor rebuilds its queue from queued turns after a restart).
    pub fn replace_queue(&self, boot_ms: u64, entries: &[QueueEntry]) -> Result<()> {
        let entries = entries.to_vec();
        self.write(move |tx| {
            tx.execute("DELETE FROM queue_entries", [])?;
            for e in &entries {
                tx.execute(
                    "INSERT INTO queue_entries(id, boot_ms, kind, agent_id, allocation_id, tier, effective_tier, enqueued_at_ms, reason, estimated_start_ms, data)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
                    params![
                        e.id.0 as i64,
                        boot_ms as i64,
                        tag(&e.kind),
                        e.agent.0,
                        e.allocation.to_string(),
                        tag(&e.tier),
                        tag(&e.effective_tier),
                        ms(e.enqueued_at),
                        e.reason.to_string(),
                        e.estimated_start.map(ms),
                        js(e)
                    ],
                )?;
            }
            Ok(())
        })
    }

    pub fn queue_entries(&self) -> Result<Vec<QueueEntry>> {
        self.read(|c| {
            let mut st = c.prepare("SELECT data FROM queue_entries ORDER BY enqueued_at_ms, id")?;
            Ok(st
                .query_map([], |r| from_js::<QueueEntry>(&r.get::<_, String>(0)?))?
                .collect::<rusqlite::Result<_>>()?)
        })
    }

    /// Upserts Governor actions by (core boot, action id): outcomes and undo
    /// change after the fact, so recent actions are written again.
    pub fn save_actions(&self, boot_ms: u64, plan_id: &str, actions: &[GovernorAction]) -> Result<()> {
        let (actions, plan_id) = (actions.to_vec(), plan_id.to_string());
        self.write(move |tx| {
            for a in &actions {
                tx.execute(
                    "INSERT INTO governor_actions(boot_ms, gov_id, plan_id, ts_ms, state, action, reason, mode, outcome, undone_at_ms, data)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
                     ON CONFLICT(boot_ms, gov_id) DO UPDATE SET outcome=excluded.outcome,
                       undone_at_ms=excluded.undone_at_ms, data=excluded.data",
                    params![
                        boot_ms as i64,
                        a.id.0 as i64,
                        plan_id,
                        ms(a.ts),
                        tag(&a.state),
                        a.kind_name(),
                        a.reason,
                        tag(&a.mode),
                        tag(&a.outcome),
                        a.undone_at.map(ms),
                        js(a)
                    ],
                )?;
            }
            Ok(())
        })
    }

    /// The most recent actions across restarts, newest first.
    pub fn actions(&self, limit: usize) -> Result<Vec<GovernorAction>> {
        self.read(move |c| {
            let mut st = c.prepare("SELECT data FROM governor_actions ORDER BY ts_ms DESC, rowid_ DESC LIMIT ?1")?;
            Ok(st
                .query_map([limit as i64], |r| from_js::<GovernorAction>(&r.get::<_, String>(0)?))?
                .collect::<rusqlite::Result<_>>()?)
        })
    }

    pub fn add_calibration(&self, c: &Calibration) -> Result<()> {
        let c = *c;
        self.write(move |tx| {
            tx.execute(
                "INSERT INTO calibrations(ts_ms, cost_per_weekly_pct, confidence, unattributed_rate, data) VALUES (?1,?2,?3,?4,?5)",
                params![ms(c.ts), c.cost_per_weekly_pct, c.confidence, c.unattributed_rate, js(&c)],
            )?;
            Ok(())
        })
    }

    pub fn latest_calibration(&self) -> Result<Option<Calibration>> {
        self.read(|c| {
            json_row(c, "SELECT data FROM calibrations ORDER BY ts_ms DESC, id DESC LIMIT 1", [], "calibration")
        })
    }

    pub fn add_forecast(&self, f: &ForecastSnapshot) -> Result<()> {
        let f = f.clone();
        self.write(move |tx| {
            tx.execute(
                "INSERT INTO forecast_snapshots(ts_ms, used, ideal, governed, error, projected, eta_ms, state, data)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![
                    ms(f.ts),
                    f.used,
                    f.ideal,
                    f.governed,
                    f.error,
                    f.projected,
                    f.eta.map(ms),
                    tag(&f.state),
                    js(&f)
                ],
            )?;
            Ok(())
        })
    }

    /// Snapshots with `ts >= since_ms`, oldest first (forecast chart, ±10 pp check).
    pub fn forecasts_since(&self, since_ms: u64) -> Result<Vec<ForecastSnapshot>> {
        self.read(move |c| {
            let mut st = c.prepare("SELECT data FROM forecast_snapshots WHERE ts_ms >= ?1 ORDER BY ts_ms, id")?;
            Ok(st
                .query_map([since_ms as i64], |r| from_js::<ForecastSnapshot>(&r.get::<_, String>(0)?))?
                .collect::<rusqlite::Result<_>>()?)
        })
    }

    // ------------------------------------------------------------ terminals

    pub fn upsert_terminal(&self, t: &Terminal) -> Result<()> {
        let t = t.clone();
        self.write(move |tx| {
            tx.execute(
                "INSERT INTO terminals(id, project_id, agent_id, cwd, shell, layout, created_at_ms) VALUES (?1,?2,?3,?4,?5,?6,?7)
                 ON CONFLICT(id) DO UPDATE SET project_id=excluded.project_id, agent_id=excluded.agent_id,
                   cwd=excluded.cwd, shell=excluded.shell, layout=excluded.layout",
                params![t.id, t.project_id, t.agent_id, p(&t.cwd), t.shell, js(&t.layout), t.created_at_ms as i64],
            )?;
            Ok(())
        })
    }

    pub fn delete_terminal(&self, id: &str) -> Result<()> {
        let id = id.to_string();
        self.write(move |tx| {
            tx.execute("DELETE FROM terminals WHERE id=?1", [id])?;
            Ok(())
        })
    }

    pub fn terminals(&self) -> Result<Vec<Terminal>> {
        self.read(|c| {
            let mut st = c.prepare(
                "SELECT id, project_id, agent_id, cwd, shell, layout, created_at_ms FROM terminals ORDER BY created_at_ms, id",
            )?;
            Ok(st
                .query_map([], |r| {
                    Ok(Terminal {
                        id: r.get(0)?,
                        project_id: r.get(1)?,
                        agent_id: r.get(2)?,
                        cwd: PathBuf::from(r.get::<_, String>(3)?),
                        shell: r.get(4)?,
                        layout: from_js(&r.get::<_, String>(5)?)?,
                        created_at_ms: r.get::<_, i64>(6)? as u64,
                    })
                })?
                .collect::<rusqlite::Result<_>>()?)
        })
    }

    // ------------------------------------------------------------ settings

    pub fn set_setting(&self, key: &str, value: &Value) -> Result<()> {
        let (key, value) = (key.to_string(), js(value));
        self.write(move |tx| {
            tx.execute(
                "INSERT INTO kv(key, value) VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![key, value],
            )?;
            Ok(())
        })
    }

    pub fn setting(&self, key: &str) -> Result<Option<Value>> {
        let key = key.to_string();
        self.read(move |c| {
            let s: Option<String> = c.query_row("SELECT value FROM kv WHERE key=?1", [key], |r| r.get(0)).optional()?;
            Ok(s.and_then(|s| serde_json::from_str(&s).ok()))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use governor::{BudgetAllocation, Tier, WorkingCalendar};

    fn store() -> (tempfile::TempDir, Store) {
        let d = tempfile::tempdir().unwrap();
        let s = Store::open(&d.path().join("orch.db")).unwrap();
        (d, s)
    }

    pub(crate) fn project(id: &str, path: &str) -> Project {
        Project {
            id: id.into(),
            name: id.into(),
            path: path.into(),
            default_branch: "main".into(),
            remotes: vec![Remote { name: "origin".into(), url: "git@example.invalid:x.git".into() }],
            worktree_root: format!("{path}.worktrees").into(),
            task_backend: serde_json::json!({"type": "local"}),
            track_orchestrator_dir: true,
            settings: serde_json::json!({}),
            created_at_ms: 1,
            last_active_at_ms: 1,
            last_fetch_at_ms: None,
            last_fetch_error: None,
            archived: false,
        }
    }

    #[test]
    fn projects_crud_touch_fetch_delete() {
        let (_d, s) = store();
        s.upsert_project(&project("demo", "/x/demo")).unwrap();
        s.upsert_project(&project("other", "/x/other")).unwrap();
        s.touch_project("demo", 50).unwrap();
        s.touch_project("demo", 10).unwrap(); // never goes back
        s.record_fetch("demo", 60, Some("offline".into())).unwrap();
        let p = s.project("demo").unwrap().unwrap();
        assert_eq!(
            (p.last_active_at_ms, p.last_fetch_at_ms, p.last_fetch_error.as_deref()),
            (50, Some(60), Some("offline"))
        );
        assert_eq!(s.project_by_path(Path::new("/x/other")).unwrap().unwrap().id, "other");
        let mut arch = s.project("other").unwrap().unwrap();
        arch.archived = true;
        s.upsert_project(&arch).unwrap();
        assert_eq!(s.projects(false).unwrap().len(), 1);
        assert_eq!(s.projects(true).unwrap().len(), 2);
        s.add_rule(RuleScope::Project, "demo", "Bash(ls:*)", RuleEffect::Allow, None).unwrap();
        s.delete_project("demo").unwrap();
        assert!(s.project("demo").unwrap().is_none());
        assert!(s.rules().unwrap().is_empty());
    }

    #[test]
    fn rules_are_revocable_and_kept() {
        let (_d, s) = store();
        let r = s.add_rule(RuleScope::Role, "solo", "mcp__orch__commit", RuleEffect::Ask, Some(3)).unwrap();
        assert_eq!(s.rules().unwrap(), vec![r.clone()]);
        assert!(s.revoke_rule(r.id).unwrap());
        assert!(!s.revoke_rule(r.id).unwrap());
        assert!(s.rules().unwrap().is_empty());
        let n: i64 = s.read(|c| Ok(c.query_row("SELECT count(*) FROM permission_rules", [], |r| r.get(0))?)).unwrap();
        assert_eq!(n, 1, "kept for the audit trail");
    }

    #[test]
    fn usage_plan_policy_governor_log() {
        let (_d, s) = store();
        s.add_usage(UsageRecord {
            ts_ms: 10,
            agent_id: Some("a1".into()),
            weekly_pct: Some(40.0),
            weekly_resets_at: Some(1_800_000_000),
            cost_estimate: Some(0.2),
            cost_delta: Some(0.1),
            provenance: "observed".into(),
            source: Some(UsageSource::Statusline),
            ..Default::default()
        });
        s.add_usage(UsageRecord {
            ts_ms: 11,
            agent_id: Some("a1".into()),
            tokens_in: Some(100),
            tokens_out: Some(20),
            cost_estimate: Some(0.05),
            provenance: "observed".into(),
            source: Some(UsageSource::Otel),
            ..Default::default()
        });
        s.flush().unwrap();
        let st = s.usage_since(0, Some(UsageSource::Statusline)).unwrap();
        assert_eq!(st.len(), 1);
        assert_eq!(st[0].weekly_pct, Some(40.0));
        assert_eq!(s.usage_since(11, None).unwrap().len(), 1);
        let tot = s.usage_totals(0).unwrap();
        assert_eq!(
            tot,
            vec![UsageTotal {
                agent_id: Some("a1".into()),
                tokens_in: 100,
                tokens_out: 20,
                cache_read: 0,
                cache_write: 0,
                cost: 0.05
            }]
        );

        let start: Timestamp = "2026-10-19T07:00:00Z".parse().unwrap();
        let plan = WeeklyPlan {
            id: "w1".into(),
            week_start: start,
            reset_at: "2026-10-26T07:00:00Z".parse().unwrap(),
            calendar: WorkingCalendar::weekdays(
                "Europe/Paris",
                governor::jiff::civil::time(9, 0, 0, 0),
                governor::jiff::civil::time(19, 0, 0, 0),
            ),
            reserve_pct: 10.0,
            off_hours_allowance_pct: 0.0,
            shadow_mode: true,
            allocations: vec![BudgetAllocation {
                id: "A".into(),
                target: "project A".into(),
                weight: 60.0,
                tier: Tier::High,
                hard_cap_pct: None,
            }],
        };
        s.save_plan(&plan, PlanStatus::Active).unwrap();
        let mut next = plan.clone();
        next.id = "w2".into();
        next.week_start = plan.reset_at;
        next.reset_at = "2026-11-02T08:00:00Z".parse().unwrap();
        s.save_plan(&next, PlanStatus::Active).unwrap();
        assert_eq!(s.active_plan().unwrap().unwrap().id, "w2");
        assert_eq!(s.previous_plan(next.week_start).unwrap().unwrap(), plan);
        let tier: String = s
            .read(|c| {
                Ok(c.query_row("SELECT tier FROM budget_allocations WHERE weekly_plan_id='w1'", [], |r| r.get(0))?)
            })
            .unwrap();
        assert_eq!(tier, "high");

        let pol = PacingPolicy::default();
        s.save_policy("default", &pol).unwrap();
        assert_eq!(s.policy("default").unwrap().unwrap(), pol);

        let mut gov = governor::Governor::new(plan.clone(), pol, start).unwrap();
        gov.pause_all(start);
        s.save_actions(1, &plan.id, gov.actions()).unwrap();
        s.save_actions(1, &plan.id, gov.actions()).unwrap(); // upsert, no duplicate
        assert_eq!(s.actions(10).unwrap().len(), gov.actions().len());
        s.add_forecast(&gov.snapshot(start)).unwrap();
        assert_eq!(s.forecasts_since(0).unwrap().len(), 1);
        s.add_calibration(&gov.calibration(start)).unwrap();
        assert!(s.latest_calibration().unwrap().is_some());
        let _ = gov.request_turn(governor::TurnRequest::task("a", "A", Tier::Normal), start);
        s.replace_queue(1, gov.queue()).unwrap();
        assert_eq!(s.queue_entries().unwrap().len(), gov.queue().len());
    }

    #[test]
    fn terminals_and_settings() {
        let (_d, s) = store();
        let t = Terminal {
            id: "t1".into(),
            project_id: Some("demo".into()),
            agent_id: None,
            cwd: "/x".into(),
            shell: "/bin/zsh".into(),
            layout: serde_json::json!({"split": "h"}),
            created_at_ms: 1,
        };
        s.upsert_terminal(&t).unwrap();
        assert_eq!(s.terminals().unwrap(), vec![t]);
        s.delete_terminal("t1").unwrap();
        assert!(s.terminals().unwrap().is_empty());
        s.set_setting("fetch.interval_min", &serde_json::json!(15)).unwrap();
        assert_eq!(s.setting("fetch.interval_min").unwrap(), Some(serde_json::json!(15)));
        assert_eq!(s.setting("nope").unwrap(), None);
    }
}
