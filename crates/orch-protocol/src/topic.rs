//! The `now` event's invalidation topics (P4.1 api-contract §3.1): what a
//! pane refetches when it arrives. One closed set, written by the core.

use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Topic {
    Agents,
    Queue,
    Usage,
    Inbox,
    Features,
    Feature(String),
    Board(String),
    Planner,
    Teams,
    Project(String),
    Worktree(String),
    Report,
    Integrations,
    Memory,
    Power,
    Banners,
    Setup,
}

impl fmt::Display for Topic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Topic::Agents => f.write_str("agents"),
            Topic::Queue => f.write_str("queue"),
            Topic::Usage => f.write_str("usage"),
            Topic::Inbox => f.write_str("inbox"),
            Topic::Features => f.write_str("features"),
            Topic::Feature(id) => write!(f, "feature:{id}"),
            Topic::Board(project) => write!(f, "board:{project}"),
            Topic::Planner => f.write_str("planner"),
            Topic::Teams => f.write_str("teams"),
            Topic::Project(id) => write!(f, "project:{id}"),
            Topic::Worktree(path) => write!(f, "worktree:{path}"),
            Topic::Report => f.write_str("report"),
            Topic::Integrations => f.write_str("integrations"),
            Topic::Memory => f.write_str("memory"),
            Topic::Power => f.write_str("power"),
            Topic::Banners => f.write_str("banners"),
            Topic::Setup => f.write_str("setup"),
        }
    }
}

impl From<Topic> for String {
    fn from(t: Topic) -> String {
        t.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_names() {
        assert_eq!(String::from(Topic::Feature("shop-checkout".into())), "feature:shop-checkout");
        assert_eq!(Topic::Worktree("/w/back-1".into()).to_string(), "worktree:/w/back-1");
        assert_eq!(Topic::Agents.to_string(), "agents");
    }
}
