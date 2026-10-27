//! P3 integrations and the MCP registry in the app (api-contract §1–2):
//! thin over the runtime (`agent_runtime::integrations`, `registry`).

use anyhow::{Result, anyhow};
use orch_secrets::Secret;
use orch_team::Scope;
use orch_team::mcp::{McpKind, McpServer};
use serde_json::{Value, json};

use crate::core::Core;

/// Where the claude.ai connectors are managed.
pub const CONNECTOR_SETTINGS_URL: &str = "https://claude.ai/settings/connectors";

impl Core {
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
        let kind = rt.db.mcp_registry(None)?.get(id).map(|s| s.kind);
        Ok(match kind {
            Some(McpKind::Http | McpKind::Sse) => json!({"open": "pane", "run": "claude /mcp", "server": id}),
            Some(McpKind::Connector) => json!({"open": "url", "url": CONNECTOR_SETTINGS_URL, "server": id}),
            _ => settings,
        })
    }
}
