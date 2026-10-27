//! Import and export of teams and roles as JSON (FR-307).

use serde::{Deserialize, Serialize};

use crate::model::{RoleDef, Team};
use crate::validate::{Issue, validate};

/// Bumped when the file shape changes; older files stay readable.
pub const FORMAT: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TeamFile {
    pub format: u32,
    pub team: Team,
    /// The roles its nodes use, so the file stands alone.
    pub roles: Vec<RoleDef>,
}

pub fn export(team: &Team, roles: &[RoleDef]) -> String {
    let used: Vec<RoleDef> = roles.iter().filter(|r| team.nodes.iter().any(|n| n.role_id == r.id)).cloned().collect();
    serde_json::to_string_pretty(&TeamFile { format: FORMAT, team: team.clone(), roles: used })
        .expect("team serializes")
}

#[derive(Debug, PartialEq)]
pub enum ImportError {
    Json(String),
    NewerFormat(u32),
    Invalid(Vec<Issue>),
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::Json(e) => write!(f, "not a team file: {e}"),
            ImportError::NewerFormat(v) => write!(f, "team file format {v} is newer than this app ({FORMAT})"),
            ImportError::Invalid(issues) => {
                write!(f, "invalid team: {}", issues.iter().map(|i| i.to_string()).collect::<Vec<_>>().join("; "))
            }
        }
    }
}

/// Parses and validates a team file. The roles it carries must cover its nodes.
pub fn import(json: &str) -> Result<TeamFile, ImportError> {
    let file: TeamFile = serde_json::from_str(json).map_err(|e| ImportError::Json(e.to_string()))?;
    if file.format > FORMAT {
        return Err(ImportError::NewerFormat(file.format));
    }
    let issues = validate(&file.team, Some(&file.roles));
    if !issues.is_empty() {
        return Err(ImportError::Invalid(issues));
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presets;

    #[test]
    fn round_trip_keeps_only_used_roles() {
        let (_, t) = presets::bugfix_squad();
        let json = export(&t, &presets::starter_roles());
        let file = import(&json).unwrap();
        assert_eq!(file.team, t);
        let mut ids: Vec<&str> = file.roles.iter().map(|r| r.id.as_str()).collect();
        ids.sort();
        assert_eq!(ids, vec!["backend", "frontend", "lead"]);
    }

    #[test]
    fn rejects_bad_files() {
        assert!(matches!(import("{"), Err(ImportError::Json(_))));
        let (roles, t) = presets::solo_dev();
        let mut file = TeamFile { format: FORMAT + 1, team: t.clone(), roles: roles.clone() };
        assert_eq!(import(&serde_json::to_string(&file).unwrap()), Err(ImportError::NewerFormat(FORMAT + 1)));
        file.format = FORMAT;
        file.roles.clear();
        assert!(matches!(import(&serde_json::to_string(&file).unwrap()), Err(ImportError::Invalid(_))));
    }
}
