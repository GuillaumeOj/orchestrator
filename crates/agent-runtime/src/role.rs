//! A Role: the reusable agent definition compiled into a per-agent config (SPEC §6.3, §7).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// `--permission-mode`, always passed explicitly. `bypassPermissions` is not
/// representable: the app never uses it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermissionMode {
    #[default]
    #[serde(rename = "manual")]
    Manual,
    #[serde(rename = "acceptEdits")]
    AcceptEdits,
    #[serde(rename = "auto")]
    Auto,
    #[serde(rename = "dontAsk")]
    DontAsk,
    #[serde(rename = "plan")]
    Plan,
}

impl PermissionMode {
    pub fn as_flag(self) -> &'static str {
        match self {
            PermissionMode::Manual => "manual",
            PermissionMode::AcceptEdits => "acceptEdits",
            PermissionMode::Auto => "auto",
            PermissionMode::DontAsk => "dontAsk",
            PermissionMode::Plan => "plan",
        }
    }

    pub fn parse(s: &str) -> Result<PermissionMode, String> {
        match s {
            "manual" | "default" => Ok(PermissionMode::Manual),
            "acceptEdits" => Ok(PermissionMode::AcceptEdits),
            "auto" => Ok(PermissionMode::Auto),
            "dontAsk" => Ok(PermissionMode::DontAsk),
            "plan" => Ok(PermissionMode::Plan),
            "bypassPermissions" => Err("bypassPermissions is never used for agents".into()),
            other => Err(format!("unknown permission mode {other:?}")),
        }
    }
}

/// Package registries an agent may reach by default (ADR-009: per-role allow-list).
pub const DEFAULT_NETWORK_ALLOW: &[&str] = &[
    "registry.npmjs.org",
    "registry.yarnpkg.com",
    "pypi.org",
    "files.pythonhosted.org",
    "crates.io",
    "index.crates.io",
    "static.crates.io",
    "proxy.golang.org",
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Role {
    pub name: String,
    /// Appended after the ORCH preamble in `--append-system-prompt`.
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub permission_mode: PermissionMode,
    /// `--strict-mcp-config`: ignore the user's other MCP servers (~25 MB less per agent).
    #[serde(default)]
    pub strict_mcp: bool,
    /// `--setting-sources` (e.g. `["project","local"]`) to skip the user's settings.
    #[serde(default)]
    pub setting_sources: Option<Vec<String>>,
    /// `mcpServers` entries for `--mcp-config`.
    #[serde(default)]
    pub mcp_servers: serde_json::Map<String, serde_json::Value>,
    #[serde(default = "default_network")]
    pub network_allow: Vec<String>,
    /// Extra `permissions.allow` rules.
    #[serde(default)]
    pub allow_rules: Vec<String>,
    /// Extra `permissions.deny` rules (on top of the generated ones).
    #[serde(default)]
    pub deny_rules: Vec<String>,
    /// Extra sandbox write allowances (e.g. a tool cache), visible in the settings.
    #[serde(default)]
    pub extra_write: Vec<PathBuf>,
}

fn default_network() -> Vec<String> {
    DEFAULT_NETWORK_ALLOW.iter().map(|s| s.to_string()).collect()
}

impl Role {
    /// "Plain Claude" for a solo session.
    pub fn solo() -> Role {
        Role {
            name: "solo".into(),
            prompt: None,
            model: None,
            effort: None,
            permission_mode: PermissionMode::Manual,
            strict_mcp: false,
            setting_sources: None,
            mcp_servers: Default::default(),
            network_allow: default_network(),
            allow_rules: vec![],
            deny_rules: vec![],
            extra_write: vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_modes() {
        for m in ["manual", "acceptEdits", "auto", "dontAsk", "plan"] {
            assert_eq!(PermissionMode::parse(m).unwrap().as_flag(), m);
        }
        assert!(PermissionMode::parse("bypassPermissions").is_err());
        assert!(serde_json::from_str::<PermissionMode>(r#""bypassPermissions""#).is_err());
    }
}
