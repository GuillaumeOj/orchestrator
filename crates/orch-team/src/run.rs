//! Team-run and task states (SPEC §6.2, FR-404, FR-502).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// `draft → spec → spec_gate → planning → executing → integrating → pr_gate →
/// done`, plus `paused`, `deferred` and `failed`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Draft,
    Spec,
    SpecGate,
    Planning,
    Executing,
    Integrating,
    PrGate,
    Done,
    Paused,
    Deferred,
    Failed,
}

impl RunState {
    pub const ALL: [RunState; 11] = [
        RunState::Draft,
        RunState::Spec,
        RunState::SpecGate,
        RunState::Planning,
        RunState::Executing,
        RunState::Integrating,
        RunState::PrGate,
        RunState::Done,
        RunState::Paused,
        RunState::Deferred,
        RunState::Failed,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            RunState::Draft => "draft",
            RunState::Spec => "spec",
            RunState::SpecGate => "spec_gate",
            RunState::Planning => "planning",
            RunState::Executing => "executing",
            RunState::Integrating => "integrating",
            RunState::PrGate => "pr_gate",
            RunState::Done => "done",
            RunState::Paused => "paused",
            RunState::Deferred => "deferred",
            RunState::Failed => "failed",
        }
    }

    /// UI label (feature card, Feature pane).
    pub fn label(self) -> &'static str {
        match self {
            RunState::Draft => "Draft",
            RunState::Spec => "Spec",
            RunState::SpecGate => "Spec gate",
            RunState::Planning => "Planning",
            RunState::Executing => "Executing",
            RunState::Integrating => "Integrating",
            RunState::PrGate => "PR gate",
            RunState::Done => "Done",
            RunState::Paused => "Paused",
            RunState::Deferred => "Deferred",
            RunState::Failed => "Failed",
        }
    }

    /// No more work happens in a finished run.
    pub fn finished(self) -> bool {
        matches!(self, RunState::Done | RunState::Failed)
    }

    /// The lead may create tasks (FR-401: only after the spec gate).
    pub fn tasks_allowed(self) -> bool {
        matches!(self, RunState::Planning | RunState::Executing | RunState::Integrating | RunState::Paused)
    }

    /// Allowed moves. Pause, defer and fail are possible from any unfinished
    /// state; a paused or deferred run resumes into the state it left
    /// (checked by the caller, which remembers it).
    pub fn can_go(self, to: RunState) -> bool {
        use RunState::*;
        if self.finished() {
            return false;
        }
        if matches!(to, Paused | Deferred | Failed) {
            return self != to;
        }
        if matches!(self, Paused | Deferred) {
            return !matches!(to, Draft);
        }
        matches!(
            (self, to),
            (Draft, Spec)
                | (Spec, SpecGate)
                | (SpecGate, Spec)
                | (SpecGate, Planning)
                | (Planning, Executing)
                | (Planning, Integrating)
                | (Executing, Integrating)
                | (Integrating, Executing)
                | (Integrating, PrGate)
                | (Executing, PrGate)
                | (PrGate, Integrating)
                | (PrGate, Done)
        )
    }
}

impl fmt::Display for RunState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for RunState {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        RunState::ALL.into_iter().find(|r| r.as_str() == s).ok_or_else(|| format!("unknown run state {s:?}"))
    }
}

/// Board columns (FR-502), the same over every backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Backlog,
    Ready,
    InProgress,
    Review,
    Done,
}

impl TaskStatus {
    pub const ALL: [TaskStatus; 5] =
        [TaskStatus::Backlog, TaskStatus::Ready, TaskStatus::InProgress, TaskStatus::Review, TaskStatus::Done];

    pub fn as_str(self) -> &'static str {
        match self {
            TaskStatus::Backlog => "backlog",
            TaskStatus::Ready => "ready",
            TaskStatus::InProgress => "in_progress",
            TaskStatus::Review => "review",
            TaskStatus::Done => "done",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            TaskStatus::Backlog => "Backlog",
            TaskStatus::Ready => "Ready",
            TaskStatus::InProgress => "In progress",
            TaskStatus::Review => "Review",
            TaskStatus::Done => "Done",
        }
    }
}

impl fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for TaskStatus {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        let s = s.trim().to_ascii_lowercase().replace([' ', '-'], "_");
        TaskStatus::ALL.into_iter().find(|t| t.as_str() == s).ok_or_else(|| format!("unknown task status {s:?}"))
    }
}

/// The dependencies of a task that are not done yet (FR-404). `done(id)`
/// answers for one dependency; an unknown id counts as not done.
pub fn blocked_by(depends_on: &[String], done: impl Fn(&str) -> bool) -> Vec<&str> {
    depends_on.iter().map(String::as_str).filter(|d| !done(d)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn happy_path_and_round_trips() {
        use RunState::*;
        let path = [Draft, Spec, SpecGate, Planning, Executing, Integrating, PrGate, Done];
        for w in path.windows(2) {
            assert!(w[0].can_go(w[1]), "{} → {}", w[0], w[1]);
        }
        assert!(!Draft.can_go(Planning));
        assert!(!Spec.can_go(Executing));
        assert!(!Done.can_go(Paused));
        assert!(Executing.can_go(Paused) && Paused.can_go(Executing));
        assert!(!Paused.can_go(Paused));
        assert!(SpecGate.can_go(Spec), "a rejected spec goes back to the lead");
        for s in RunState::ALL {
            assert_eq!(s.as_str().parse::<RunState>().unwrap(), s);
        }
        assert!(!SpecGate.tasks_allowed() && Planning.tasks_allowed());
    }

    #[test]
    fn task_status_parsing_and_deps() {
        assert_eq!("In progress".parse::<TaskStatus>().unwrap(), TaskStatus::InProgress);
        assert_eq!("in-progress".parse::<TaskStatus>().unwrap(), TaskStatus::InProgress);
        assert!("doing".parse::<TaskStatus>().is_err());
        let deps = vec!["1".to_string(), "2".to_string()];
        assert_eq!(blocked_by(&deps, |d| d == "1"), vec!["2"]);
        assert!(blocked_by(&deps, |_| true).is_empty());
    }
}
