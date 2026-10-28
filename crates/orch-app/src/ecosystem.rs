//! P3 integrations, the MCP registry and weekly reports in the app
//! (api-contract §1–2, §5): thin over the runtime (`agent_runtime::integrations`,
//! `registry`, `report`).

use anyhow::{Result, anyhow};
use orch_secrets::Secret;
use orch_team::Scope;
use orch_team::mcp::{McpKind, McpServer};
use serde_json::{Value, json};

use crate::core::Core;

/// Where the claude.ai connectors are managed.
pub const CONNECTOR_SETTINGS_URL: &str = "https://claude.ai/settings/connectors";

/// A numeric id the UI may send as a string (`"12"`, `"T-12"`) or a number.
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(untagged)]
pub enum IdArg {
    Num(u64),
    Str(String),
}

impl IdArg {
    pub fn parse(&self) -> Result<u64> {
        match self {
            IdArg::Num(n) => Ok(*n),
            IdArg::Str(s) => {
                let t = s.trim();
                t.strip_prefix("T-").unwrap_or(t).parse().map_err(|_| anyhow!("invalid id {s:?}"))
            }
        }
    }
}

/// `open_url`: only http(s) URLs, with a host and no whitespace or control
/// characters (the URL goes to `/usr/bin/open` as one argument).
pub fn checked_url(url: &str) -> Result<&str> {
    let u = url.trim();
    let rest = u.strip_prefix("https://").or_else(|| u.strip_prefix("http://"));
    let ok = rest.is_some_and(|r| {
        let host = r.split(['/', '?', '#']).next().unwrap_or("");
        !host.is_empty() && !host.starts_with('-')
    }) && !u.chars().any(|c| c.is_whitespace() || c.is_control());
    if !ok {
        return Err(anyhow!("only http and https links can be opened"));
    }
    Ok(u)
}

impl Core {
    /// `open_url`: an http(s) link in the default browser (D3: the claude.ai
    /// connector settings for a connector's reconnect).
    pub fn open_url(&self, url: &str) -> Result<()> {
        let u = checked_url(url)?;
        let st = std::process::Command::new("/usr/bin/open").arg(u).status()?;
        if !st.success() {
            return Err(anyhow!("open exited with {st}"));
        }
        Ok(())
    }

    /// `reports_list` (api-contract §5, FR-818).
    pub fn reports_list(&self) -> Result<Value> {
        self.runtime()?.reports_list()
    }

    /// `report_view`: a stored week's report, or `current`.
    pub fn report_view(&self, id: &str) -> Result<Value> {
        self.runtime()?.report_view(id)
    }

    /// `queue_off_hours` (FR-816, D4): `{ starts_at }`.
    pub fn queue_off_hours(&self, entry: Option<u64>, task: Option<u64>, off: bool) -> Result<Value> {
        let t = self.runtime()?.queue_off_hours(entry, task, off)?;
        Ok(json!({"starts_at": t.to_string()}))
    }

    /// An MCP env secret, Keychain account `mcp:<account>`; never logged or returned.
    pub fn mcp_secret_set(&self, account: &str, value: Secret) -> Result<()> {
        self.runtime()?.mcp_secret_set(account, value)
    }

    pub fn mcp_secret_delete(&self, account: &str) -> Result<()> {
        self.runtime()?.mcp_secret_delete(account)
    }

    pub fn mcp_secrets_status(&self, accounts: &[String]) -> Result<Value> {
        Ok(self.runtime()?.mcp_secrets_status(accounts))
    }

    pub fn integrations_view(&self) -> Result<Value> {
        Ok(self.runtime()?.integrations_json())
    }

    /// The secret goes straight to the Keychain; it is never logged or returned.
    pub fn integration_secret_set(&self, id: &str, secret: Secret) -> Result<Value> {
        self.runtime()?.integration_secret_set(id, secret)
    }

    pub fn integration_secret_remove(&self, id: &str) -> Result<Value> {
        self.runtime()?.integration_secret_remove(id)
    }

    pub fn integration_test(&self, id: &str) -> Result<Value> {
        self.runtime()?.integration_test(id)
    }

    pub fn mcp_registry_view(&self, project: Option<&str>) -> Result<Value> {
        self.runtime()?.mcp_registry_json(project)
    }

    pub fn mcp_server_save(&self, server: McpServer, scope: Scope) -> Result<Value> {
        Ok(json!(self.runtime()?.mcp_server_save(server, &scope)?))
    }

    pub fn mcp_server_delete(&self, id: &str, scope: Scope) -> Result<Value> {
        self.runtime()?.mcp_server_delete(id, &scope)
    }

    pub fn mcp_server_test(&self, id: &str, project: Option<&str>) -> Result<Value> {
        Ok(json!(self.runtime()?.mcp_server_test(id, project)?))
    }

    /// What the UI does for an `mcp_needs_auth` alert's `open` or `reconnect`
    /// (decisions D3): `{"open": "settings", "server"}`, `{"open": "pane",
    /// "run": "claude /mcp"}` (http/sse: Claude Code's own sign-in) or
    /// `{"open": "url", "url"}` (a claude.ai connector). The item stays open:
    /// it closes when the server is seen connected again.
    pub fn mcp_alert_action(&self, payload: &Value, action: &str) -> Result<Value> {
        let id = payload["server"].as_str().ok_or_else(|| anyhow!("not an MCP alert"))?;
        let settings = json!({"open": "settings", "server": id});
        if action == "open" {
            return Ok(settings);
        }
        let rt = self.runtime()?;
        // The alert's project sees its own override first, then the global entry.
        let kind = rt.db.mcp_registry(payload["project"].as_str())?.get(id).map(|s| s.kind);
        Ok(match kind {
            Some(McpKind::Http | McpKind::Sse) => json!({"open": "pane", "run": "claude /mcp", "server": id}),
            Some(McpKind::Connector) => json!({"open": "url", "url": CONNECTOR_SETTINGS_URL, "server": id}),
            _ => settings,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_come_as_strings_or_numbers() {
        let p = |v: serde_json::Value| serde_json::from_value::<IdArg>(v).unwrap().parse();
        assert_eq!(p(json!("12")).unwrap(), 12);
        assert_eq!(p(json!("T-7")).unwrap(), 7);
        assert_eq!(p(json!(3)).unwrap(), 3);
        assert!(p(json!("x")).is_err());
    }

    #[test]
    fn only_http_links_open() {
        assert_eq!(
            checked_url(" https://claude.ai/settings/connectors ").unwrap(),
            "https://claude.ai/settings/connectors"
        );
        assert!(checked_url("http://localhost:3000/x?y=1").is_ok());
        for bad in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "https://",
            "-a Terminal",
            "https://a b",
            "https://-x",
            "/Applications/X.app",
            "HTTPS://x",
        ] {
            assert!(checked_url(bad).is_err(), "{bad}");
        }
    }
}
