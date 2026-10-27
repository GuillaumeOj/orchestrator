//! The MCP server registry (FR-1202, FR-601): named servers a role uses by
//! id (`RoleDef::mcp_refs`), global with per-project overrides, resolved at
//! compile time into the agent's `mcpServers`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::model::RoleDef;

/// The `env` value prefix naming a Keychain account (orch-secrets `KEYCHAIN_PREFIX`).
pub const KEYCHAIN_PREFIX: &str = "keychain:";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpKind {
    Stdio,
    Http,
    Sse,
    /// A claude.ai connector, loaded by Claude Code from the user's login: it
    /// can't be put in `--mcp-config`, and `--strict-mcp-config` hides it.
    Connector,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DesignTool {
    Penpot,
    Figma,
}

/// FR-604 auth status (also the integrations' status).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpStatus {
    Connected,
    NeedsAuth,
    Error,
    NotConfigured,
    Checking,
    #[default]
    Unknown,
}

impl McpStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            McpStatus::Connected => "connected",
            McpStatus::NeedsAuth => "needs_auth",
            McpStatus::Error => "error",
            McpStatus::NotConfigured => "not_configured",
            McpStatus::Checking => "checking",
            McpStatus::Unknown => "unknown",
        }
    }

    pub fn parse(s: &str) -> McpStatus {
        serde_json::from_value(Value::String(s.into())).unwrap_or_default()
    }
}

/// A role node using a server (filled by the view).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsedBy {
    pub team: String,
    pub node: String,
    pub role: String,
}

/// One registry entry (api-contract §2 `McpServer`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct McpServer {
    /// `[a-z0-9-]+`: the key in `mcpServers` and in `RoleDef::mcp_refs`.
    pub id: String,
    pub name: String,
    pub kind: McpKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Values may be `keychain:<account>`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// The opt-in to resolve `keychain:` env values into the agent's
    /// `mcp.json` (the agent can then read the token). Off: they are refused.
    #[serde(default)]
    pub allow_secret_env: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connector_name: Option<String>,
    #[serde(default)]
    pub design_tool: Option<DesignTool>,
    // ---- status and usage: filled by the core, ignored on save.
    #[serde(default)]
    pub status: McpStatus,
    #[serde(default)]
    pub detail: Option<String>,
    #[serde(default)]
    pub checked_at_ms: Option<u64>,
    #[serde(default)]
    pub used_by: Vec<UsedBy>,
}

/// `[a-z0-9-]+`, up to 40, not `orch` (the orchestrator's own server).
pub fn valid_server_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 40
        && id != crate::compile::ORCH_SERVER
        && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !id.starts_with('-')
}

impl McpServer {
    pub fn new(id: &str, name: &str, kind: McpKind) -> McpServer {
        McpServer {
            id: id.into(),
            name: name.into(),
            kind,
            command: None,
            args: vec![],
            url: None,
            env: BTreeMap::new(),
            allow_secret_env: false,
            connector_name: None,
            design_tool: None,
            status: McpStatus::Unknown,
            detail: None,
            checked_at_ms: None,
            used_by: vec![],
        }
    }

    /// The `penpot` entry every registry starts with (editable).
    pub fn penpot_seed() -> McpServer {
        let mut s = McpServer::new("penpot", "Penpot", McpKind::Connector);
        s.connector_name = Some("claude.ai Penpot".into());
        s.design_tool = Some(DesignTool::Penpot);
        s
    }

    /// The Keychain accounts its `env` names.
    pub fn keychain_accounts(&self) -> Vec<&str> {
        self.env.values().filter_map(|v| keychain_account(v)).collect()
    }

    /// Checks an entry before it is saved.
    pub fn validate(&self) -> Result<(), String> {
        if !valid_server_id(&self.id) {
            return Err(format!("{:?} is not a valid server id (a-z, 0-9 and -, up to 40, not `orch`)", self.id));
        }
        if self.name.trim().is_empty() {
            return Err("the server needs a name".into());
        }
        let nonempty = |o: &Option<String>| o.as_deref().is_some_and(|s| !s.trim().is_empty());
        match self.kind {
            McpKind::Stdio if !nonempty(&self.command) => return Err("a stdio server needs a command".into()),
            McpKind::Http | McpKind::Sse => {
                let url = self.url.as_deref().unwrap_or("").trim();
                if !(url.starts_with("https://") || url.starts_with("http://")) {
                    return Err("an http or sse server needs an http(s) URL".into());
                }
            }
            McpKind::Connector if !nonempty(&self.connector_name) => {
                return Err("a connector needs the claude.ai connector's name".into());
            }
            _ => {}
        }
        for (k, v) in &self.env {
            if k.is_empty() || !k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return Err(format!("invalid env name {k:?}"));
            }
            if v.starts_with(KEYCHAIN_PREFIX) && keychain_account(v).is_none() {
                return Err(format!("{k}: `keychain:` needs an account name"));
            }
        }
        Ok(())
    }

    /// The `mcpServers` entry (keychain refs left as they are: the runtime
    /// resolves them at spawn). `None` for a connector.
    pub fn mcp_entry(&self) -> Option<Value> {
        let mut e = match self.kind {
            McpKind::Stdio => json!({"type": "stdio", "command": self.command, "args": self.args}),
            McpKind::Http => json!({"type": "http", "url": self.url}),
            McpKind::Sse => json!({"type": "sse", "url": self.url}),
            McpKind::Connector => return None,
        };
        if !self.env.is_empty() {
            e["env"] = json!(self.env);
        }
        Some(e)
    }

    /// The server name in tool names (`mcp__<prefix>__<tool>`): the id, or
    /// for a connector `claude_ai_<name>` (Claude Code replaces what is not
    /// a letter, digit, `_` or `-` with `_`).
    pub fn tool_prefix(&self) -> String {
        match (self.kind, self.connector_name.as_deref()) {
            (McpKind::Connector, Some(n)) => tool_safe(n),
            _ => self.id.clone(),
        }
    }

    /// Does the tool `mcp__<server>__<tool>` belong to this entry? A
    /// connector also matches a longer connector name ("claude.ai Penpot"
    /// matches "claude.ai Penpot Perso").
    pub fn owns_tool_server(&self, server: &str) -> bool {
        let p = self.tool_prefix();
        match self.kind {
            McpKind::Connector => server == p || server.strip_prefix(&p).is_some_and(|rest| rest.starts_with('_')),
            _ => server == p,
        }
    }
}

/// How Claude Code turns a server name into the tool-name segment.
pub fn tool_safe(name: &str) -> String {
    name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect()
}

pub fn keychain_account(v: &str) -> Option<&str> {
    v.strip_prefix(KEYCHAIN_PREFIX).map(str::trim).filter(|a| !a.is_empty())
}

/// The registry as one project sees it: the global entries, each replaced by
/// the project's override of the same id, plus the project's own.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Registry {
    pub global: BTreeMap<String, McpServer>,
    pub project: BTreeMap<String, McpServer>,
}

impl Registry {
    /// Project override > global.
    pub fn get(&self, id: &str) -> Option<&McpServer> {
        self.project.get(id).or_else(|| self.global.get(id))
    }

    pub fn effective(&self) -> Vec<&McpServer> {
        let mut ids: Vec<&String> = self.global.keys().chain(self.project.keys()).collect();
        ids.sort();
        ids.dedup();
        ids.into_iter().filter_map(|id| self.get(id)).collect()
    }
}

/// What a role's `mcp_refs` compile into.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Resolved {
    /// Added to the role's raw `mcp_servers` (a ref wins over a raw entry of the same name).
    pub servers: Map<String, Value>,
    /// Servers whose `keychain:` env values may be resolved (their opt-in is on).
    pub secret_env: Vec<String>,
    /// Connectors used: they come from the user's login, not `--mcp-config`.
    pub connectors: Vec<String>,
}

/// Resolves `refs` against `reg`. Errors: an unknown ref, a `keychain:` env
/// value without the entry's opt-in, a connector with `--strict-mcp-config`.
pub fn resolve(refs: &[String], strict_mcp: bool, reg: &Registry) -> Result<Resolved, String> {
    let mut out = Resolved::default();
    for id in refs {
        let s = reg.get(id).ok_or_else(|| format!("mcp ref `{id}` is not in the registry"))?;
        if s.kind == McpKind::Connector {
            if strict_mcp {
                return Err(format!(
                    "connector `{id}` with strict MCP: --strict-mcp-config hides claude.ai connectors; turn strict MCP off"
                ));
            }
            out.connectors.push(id.clone());
            continue;
        }
        if !s.keychain_accounts().is_empty() {
            if !s.allow_secret_env {
                return Err(format!(
                    "mcp server `{id}` reads a Keychain secret into its env: allow it on the server first (the agent \
                     can then read the token)"
                ));
            }
            out.secret_env.push(id.clone());
        }
        if let Some(e) = s.mcp_entry() {
            out.servers.insert(id.clone(), e);
        }
    }
    Ok(out)
}

/// A role's registry problems for the inspector and `teams_view`.
pub fn role_issues(role: &RoleDef, reg: &Registry) -> Vec<String> {
    let mut out = Vec::new();
    for id in &role.mcp_refs {
        match reg.get(id) {
            None => out.push(format!("mcp ref `{id}` not in the registry")),
            Some(s) if s.kind == McpKind::Connector && role.strict_mcp => {
                out.push(format!("connector `{id}` with strict MCP"))
            }
            Some(s) if !s.allow_secret_env && !s.keychain_accounts().is_empty() => {
                out.push(format!("mcp server `{id}` uses a Keychain secret without the opt-in"))
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stdio(id: &str) -> McpServer {
        let mut s = McpServer::new(id, id, McpKind::Stdio);
        s.command = Some("npx".into());
        s.args = vec!["-y".into(), "srv".into()];
        s
    }

    #[test]
    fn validation() {
        assert!(stdio("figma-local").validate().is_ok());
        assert!(stdio("Figma").validate().is_err());
        assert!(stdio("orch").validate().is_err(), "the orchestrator's own name");
        let mut h = McpServer::new("h", "H", McpKind::Http);
        assert!(h.validate().is_err());
        h.url = Some("https://mcp.example.com/mcp".into());
        assert!(h.validate().is_ok());
        let mut c = McpServer::new("c", "C", McpKind::Connector);
        assert!(c.validate().is_err());
        c.connector_name = Some("claude.ai Figma".into());
        assert!(c.validate().is_ok());
        let mut e = stdio("e");
        e.env.insert("TOKEN".into(), "keychain:".into());
        assert!(e.validate().is_err());
        assert!(McpServer::penpot_seed().validate().is_ok());
    }

    #[test]
    fn wire_shape() {
        let v = serde_json::to_value(McpServer::penpot_seed()).unwrap();
        assert_eq!(v["kind"], "connector");
        assert_eq!(v["design_tool"], "penpot");
        assert_eq!(v["status"], "unknown");
        assert_eq!(v["allow_secret_env"], false);
        // The UI may send it without status or usage.
        let s: McpServer =
            serde_json::from_value(json!({"id": "x", "name": "X", "kind": "http", "url": "https://x"})).unwrap();
        assert_eq!((s.status, s.design_tool, s.allow_secret_env), (McpStatus::Unknown, None, false));
        assert_eq!(McpStatus::parse("needs_auth"), McpStatus::NeedsAuth);
        assert_eq!(McpStatus::parse("??"), McpStatus::Unknown);
    }

    #[test]
    fn resolve_project_override_wins_and_refuses_what_cant_work() {
        let mut reg = Registry::default();
        reg.global.insert("figma".into(), stdio("figma"));
        let mut over = McpServer::new("figma", "Figma (project)", McpKind::Http);
        over.url = Some("https://figma.example/mcp".into());
        reg.project.insert("figma".into(), over);
        reg.global.insert("penpot".into(), McpServer::penpot_seed());
        let r = resolve(&["figma".into(), "penpot".into()], false, &reg).unwrap();
        assert_eq!(r.servers["figma"], json!({"type": "http", "url": "https://figma.example/mcp"}));
        assert!(!r.servers.contains_key("penpot"), "a connector is not in --mcp-config");
        assert_eq!(r.connectors, ["penpot"]);
        assert!(resolve(&["penpot".into()], true, &reg).unwrap_err().contains("strict MCP"));
        assert!(resolve(&["nope".into()], false, &reg).unwrap_err().contains("not in the registry"));

        let mut sec = stdio("linear-mcp");
        sec.env.insert("LINEAR_API_KEY".into(), "keychain:linear".into());
        reg.global.insert("linear-mcp".into(), sec.clone());
        assert!(resolve(&["linear-mcp".into()], false, &reg).unwrap_err().contains("allow it"));
        sec.allow_secret_env = true;
        reg.global.insert("linear-mcp".into(), sec);
        let r = resolve(&["linear-mcp".into()], false, &reg).unwrap();
        assert_eq!(r.secret_env, ["linear-mcp"]);
        // Still the reference: the secret is read at spawn, never stored.
        assert_eq!(r.servers["linear-mcp"]["env"]["LINEAR_API_KEY"], "keychain:linear");
    }

    #[test]
    fn role_issues_for_the_inspector() {
        let mut reg = Registry::default();
        reg.global.insert("penpot".into(), McpServer::penpot_seed());
        let mut r = RoleDef::new("ui", "UI");
        r.mcp_refs = vec!["penpot".into(), "ghost".into()];
        r.strict_mcp = true;
        assert_eq!(role_issues(&r, &reg), ["connector `penpot` with strict MCP", "mcp ref `ghost` not in the registry"]);
    }

    #[test]
    fn connector_tool_names() {
        let p = McpServer::penpot_seed();
        assert_eq!(p.tool_prefix(), "claude_ai_Penpot");
        assert!(p.owns_tool_server("claude_ai_Penpot"));
        assert!(p.owns_tool_server("claude_ai_Penpot_Perso"));
        assert!(!p.owns_tool_server("claude_ai_Penpotx"));
        assert!(stdio("figma").owns_tool_server("figma"));
        assert!(!stdio("figma").owns_tool_server("figma_x"));
    }
}
