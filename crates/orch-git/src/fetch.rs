//! Throttled background fetch (FR-103): only projects active in the last 7
//! days, at a configurable interval, paused on battery below 20 % and when
//! offline. On demand fetches skip the throttle but not the offline check.

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use anyhow::Result;
use orch_store::{Project, Store};
use serde::{Deserialize, Serialize};

use crate::git::git_deadline;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FetchPolicy {
    pub interval: Duration,
    pub active_within: Duration,
    /// Pause when on battery below this charge.
    pub battery_min_pct: u8,
    pub deadline: Duration,
}

impl Default for FetchPolicy {
    fn default() -> Self {
        FetchPolicy {
            interval: Duration::from_secs(15 * 60),
            active_within: Duration::from_secs(7 * 24 * 3600),
            battery_min_pct: 20,
            deadline: Duration::from_secs(120),
        }
    }
}

/// Power and network state (behind a trait: NFR-6, and for tests).
pub trait Conditions: Send + Sync {
    /// `Some(pct)` when running on battery.
    fn battery(&self) -> Option<u8>;
    fn online(&self) -> bool;
}

/// macOS: `pmset -g batt` and a default route.
pub struct SystemConditions;

/// Parses `pmset -g batt`: `Some(pct)` only when drawing from the battery.
pub fn parse_pmset(out: &str) -> Option<u8> {
    if !out.contains("'Battery Power'") {
        return None;
    }
    let pct = out.lines().find_map(|l| l.split('\t').nth(1)?.split('%').next()?.trim().parse::<u8>().ok());
    pct.or(Some(0))
}

impl Conditions for SystemConditions {
    fn battery(&self) -> Option<u8> {
        let out = Command::new("pmset").args(["-g", "batt"]).output().ok()?;
        parse_pmset(&String::from_utf8_lossy(&out.stdout))
    }

    fn online(&self) -> bool {
        Command::new("route")
            .args(["-n", "get", "default"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum FetchOutcome {
    Fetched,
    Failed { error: String },
    Skipped { reason: String },
}

/// Why nothing should be fetched right now, if anything.
pub fn blocked(policy: &FetchPolicy, c: &dyn Conditions) -> Option<String> {
    if !c.online() {
        return Some("offline".into());
    }
    match c.battery() {
        Some(p) if p < policy.battery_min_pct => Some(format!("on battery at {p} %")),
        _ => None,
    }
}

/// Projects due for a background fetch at `now_ms`.
pub fn due<'a>(projects: &'a [Project], now_ms: u64, policy: &FetchPolicy) -> Vec<&'a Project> {
    let active = policy.active_within.as_millis() as u64;
    let every = policy.interval.as_millis() as u64;
    projects
        .iter()
        .filter(|p| !p.archived && !p.remotes.is_empty())
        .filter(|p| now_ms.saturating_sub(p.last_active_at_ms) <= active)
        .filter(|p| p.last_fetch_at_ms.is_none_or(|t| now_ms.saturating_sub(t) >= every))
        .collect()
}

/// `git fetch --all --prune` with the user's own credentials (the core runs
/// outside the sandbox), no prompts, and a deadline.
pub fn fetch_repo(repo: &Path, deadline: Duration) -> Result<()> {
    git_deadline(repo, &["fetch", "--all", "--prune", "--quiet", "--no-write-fetch-head"], deadline)?;
    Ok(())
}

/// One background pass: fetches every due project, records the result.
pub fn tick(store: &Store, policy: &FetchPolicy, c: &dyn Conditions, now_ms: u64) -> Result<Vec<(String, FetchOutcome)>> {
    let projects = store.projects(false)?;
    let due = due(&projects, now_ms, policy);
    if due.is_empty() {
        return Ok(vec![]);
    }
    if let Some(reason) = blocked(policy, c) {
        return Ok(due.iter().map(|p| (p.id.clone(), FetchOutcome::Skipped { reason: reason.clone() })).collect());
    }
    Ok(due.into_iter().map(|p| (p.id.clone(), fetch_project(store, p, policy, now_ms))).collect())
}

/// Fetches one project now (on demand), unless offline.
pub fn fetch_now(store: &Store, project: &Project, policy: &FetchPolicy, c: &dyn Conditions) -> FetchOutcome {
    if !c.online() {
        return FetchOutcome::Skipped { reason: "offline".into() };
    }
    fetch_project(store, project, policy, orch_store::now_ms())
}

fn fetch_project(store: &Store, p: &Project, policy: &FetchPolicy, now_ms: u64) -> FetchOutcome {
    let r = fetch_repo(&p.path, policy.deadline);
    let err = r.as_ref().err().map(|e| format!("{e:#}"));
    let _ = store.record_fetch(&p.id, now_ms, err.clone());
    match err {
        None => FetchOutcome::Fetched,
        Some(error) => FetchOutcome::Failed { error },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{commit_file, git as tgit, scratch_repo};

    struct Fake {
        battery: Option<u8>,
        online: bool,
    }

    impl Conditions for Fake {
        fn battery(&self) -> Option<u8> {
            self.battery
        }
        fn online(&self) -> bool {
            self.online
        }
    }

    #[test]
    fn pmset() {
        assert_eq!(parse_pmset("Now drawing from 'AC Power'\n -InternalBattery-0 (id=1)\t100%; charged;"), None);
        assert_eq!(
            parse_pmset("Now drawing from 'Battery Power'\n -InternalBattery-0 (id=1)\t17%; discharging; 1:02 remaining"),
            Some(17)
        );
    }

    #[test]
    fn throttle_battery_offline_and_real_fetch() {
        let (d, repo) = scratch_repo();
        let remote = d.path().join("remote.git");
        tgit(d.path(), &["clone", "-q", "--bare", repo.to_str().unwrap(), remote.to_str().unwrap()]);
        tgit(&repo, &["remote", "add", "origin", remote.to_str().unwrap()]);
        // Someone else pushes to the remote.
        let other = d.path().join("other");
        tgit(d.path(), &["clone", "-q", remote.to_str().unwrap(), other.to_str().unwrap()]);
        commit_file(&other, "x.txt", "x", "remote work");
        tgit(&other, &["push", "-q", "origin", "main"]);

        let store = Store::open(&d.path().join("orch.db")).unwrap();
        let p = crate::project::register(&store, &repo).unwrap();
        let now = orch_store::now_ms();
        let pol = FetchPolicy::default();
        let ok = Fake { battery: None, online: true };
        assert_eq!(tick(&store, &pol, &Fake { battery: Some(10), online: true }, now).unwrap()[0].1,
            FetchOutcome::Skipped { reason: "on battery at 10 %".into() });
        assert_eq!(tick(&store, &pol, &Fake { battery: None, online: false }, now).unwrap()[0].1,
            FetchOutcome::Skipped { reason: "offline".into() });
        assert!(tick(&store, &pol, &Fake { battery: Some(80), online: true }, now).unwrap()[0].1 == FetchOutcome::Fetched);
        assert!(crate::git::git(&repo, &["rev-parse", "origin/main"]).is_ok());
        // Throttled until the interval passes; inactive projects are skipped.
        assert!(tick(&store, &pol, &ok, now + 1000).unwrap().is_empty());
        assert_eq!(tick(&store, &pol, &ok, now + pol.interval.as_millis() as u64).unwrap().len(), 1);
        let later = now + pol.active_within.as_millis() as u64 + pol.interval.as_millis() as u64 * 3;
        assert!(tick(&store, &pol, &ok, later).unwrap().is_empty());
        // On demand.
        assert_eq!(fetch_now(&store, &p, &pol, &ok), FetchOutcome::Fetched);
        assert!(store.project(&p.id).unwrap().unwrap().last_fetch_error.is_none());
    }
}
