//! Integrations (FR-1203): GitHub through `gh`, Linear and Notion through a
//! token in the Keychain (tested from the core: agents never call them),
//! Penpot and Figma through the MCP registry entries tagged with their
//! design tool. Only the status is stored; the UI sends a token once and
//! never reads it back (`has_secret`).

use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use orch_protocol::now_ms;
use orch_secrets::Secret;
use orch_store::IntegrationRow;
use orch_team::mcp::{DesignTool, McpStatus};
use serde_json::{Value, json};

use crate::net::{HttpRequest, HttpResponse, host_of};
use crate::runtime::Runtime;

/// The integrations, in display order.
pub const IDS: &[&str] = &["github", "linear", "notion", "penpot", "figma"];

pub const LINEAR_URL: &str = "https://api.linear.app/graphql";
pub const NOTION_URL: &str = "https://api.notion.com/v1/users/me";
pub const NOTION_VERSION: &str = "2022-06-28";
const DEADLINE: Duration = Duration::from_secs(15);

fn name_of(id: &str) -> String {
    match id {
        "github" => "GitHub".into(),
        "linear" => "Linear".into(),
        "notion" => "Notion".into(),
        "penpot" => "Penpot".into(),
        "figma" => "Figma".into(),
        other => other.into(),
    }
}

/// Linear and Notion keep a token in the Keychain; GitHub has `gh`, the
/// design tools their MCP servers.
pub fn needs_secret(id: &str) -> bool {
    !matches!(id, "github" | "penpot" | "figma")
}

fn design_tool(id: &str) -> Option<DesignTool> {
    match id {
        "penpot" => Some(DesignTool::Penpot),
        "figma" => Some(DesignTool::Figma),
        _ => None,
    }
}

/// `(status, detail, account)` of a test.
type Outcome = (McpStatus, Option<String>, Option<String>);

/// `gh auth status` output → the account it is logged in as.
pub fn gh_account(text: &str) -> Option<String> {
    for line in text.lines() {
        for marker in [" account ", "Logged in to github.com as "] {
            if line.contains("Logged in to") && line.contains(marker.trim_start()) {
                let after = line.split(marker).nth(1)?;
                let name = after.split_whitespace().next()?.trim_matches(|c: char| !c.is_alphanumeric() && c != '-');
                if !name.is_empty() {
                    return Some(name.to_string());
                }
            }
        }
    }
    None
}

/// Linear's GraphQL answer to `{ viewer { name } organization { name } }`.
pub fn linear_outcome(r: &HttpResponse) -> Outcome {
    let v: Value = serde_json::from_str(&r.body).unwrap_or(Value::Null);
    let auth_error =
        v["errors"].as_array().is_some_and(|es| es.iter().any(|e| e["extensions"]["code"] == "AUTHENTICATION_ERROR"));
    if r.status == 401 || r.status == 403 || auth_error {
        return (McpStatus::NeedsAuth, Some(format!("{} from api.linear.app", r.status)), None);
    }
    if r.status == 200
        && let Some(viewer) = v["data"]["viewer"]["name"].as_str()
    {
        let org = v["data"]["organization"]["name"].as_str().map(str::to_string);
        return (McpStatus::Connected, Some(format!("logged in as {viewer}")), org);
    }
    let msg = v["errors"][0]["message"].as_str().map(|m| format!(": {m}")).unwrap_or_default();
    (McpStatus::Error, Some(format!("{} from api.linear.app{msg}", r.status)), None)
}

/// Notion's `GET /v1/users/me` (the integration's bot user).
pub fn notion_outcome(r: &HttpResponse) -> Outcome {
    let v: Value = serde_json::from_str(&r.body).unwrap_or(Value::Null);
    match r.status {
        200 => {
            let name = v["name"].as_str().unwrap_or("the integration");
            let ws = v["bot"]["workspace_name"].as_str().map(str::to_string);
            (McpStatus::Connected, Some(format!("connected as {name}")), ws)
        }
        401 | 403 => (McpStatus::NeedsAuth, Some(format!("{} from api.notion.com", r.status)), None),
        code => {
            let msg = v["message"].as_str().map(|m| format!(": {m}")).unwrap_or_default();
            (McpStatus::Error, Some(format!("{code} from api.notion.com{msg}")), None)
        }
    }
}

impl Runtime {
    fn integration_row(&self, id: &str) -> IntegrationRow {
        self.db.integrations().unwrap_or_default().into_iter().find(|r| r.id == id).unwrap_or(IntegrationRow {
            id: id.into(),
            status: McpStatus::Unknown,
            detail: None,
            account: None,
            checked_at_ms: None,
        })
    }

    /// api-contract §1 `Integration`.
    pub fn integration_json(&self, id: &str) -> Value {
        let needs = needs_secret(id);
        let has_secret = needs && self.secrets().has(id);
        let row = match design_tool(id) {
            Some(tool) => self.design_tool_row(id, tool),
            None => self.integration_row(id),
        };
        // No token, nothing to test: not configured, whatever was stored.
        let (status, detail) =
            if needs && !has_secret { (McpStatus::NotConfigured, None) } else { (row.status, row.detail) };
        json!({
            "id": id, "name": name_of(id), "status": status, "detail": detail,
            "has_secret": has_secret, "needs_secret": needs,
            "checked_at_ms": row.checked_at_ms, "account": row.account,
        })
    }

    /// Penpot and Figma: the registry servers tagged with the tool, in any scope.
    fn design_tool_row(&self, id: &str, tool: DesignTool) -> IntegrationRow {
        let servers: Vec<_> = self
            .db
            .mcp_servers_all()
            .unwrap_or_default()
            .into_iter()
            .filter(|(_, s)| s.design_tool == Some(tool))
            .map(|(_, s)| s)
            .collect();
        let pick = [McpStatus::NeedsAuth, McpStatus::Error, McpStatus::Connected, McpStatus::Checking];
        let status = if servers.is_empty() {
            McpStatus::NotConfigured
        } else {
            pick.into_iter().find(|p| servers.iter().any(|s| s.status == *p)).unwrap_or(McpStatus::Unknown)
        };
        let mut names: Vec<&str> = servers.iter().map(|s| s.name.as_str()).collect();
        names.dedup();
        IntegrationRow {
            id: id.into(),
            status,
            detail: (!names.is_empty()).then(|| format!("MCP: {}", names.join(", "))),
            account: None,
            checked_at_ms: servers.iter().filter_map(|s| s.checked_at_ms).max(),
        }
    }

    pub fn integrations_json(&self) -> Value {
        json!({"items": IDS.iter().map(|id| self.integration_json(id)).collect::<Vec<_>>()})
    }

    /// Stores a token in the Keychain (never anywhere else), then tests it.
    /// An id that is not an integration is a Keychain account for MCP env
    /// values (`keychain:<id>`): stored, nothing to test.
    pub fn integration_secret_set(&self, id: &str, secret: Secret) -> Result<Value> {
        if !needs_secret(id) {
            bail!("{} doesn't take a token here", name_of(id));
        }
        if secret.expose().trim().is_empty() {
            bail!("the token is empty");
        }
        let secret = Secret::new(secret.expose().trim());
        self.secrets().set(id, &secret)?;
        self.event("-", "integration_secret_set", json!({"id": id}));
        if IDS.contains(&id) { self.integration_test(id) } else { Ok(self.integration_json(id)) }
    }

    pub fn integration_secret_remove(&self, id: &str) -> Result<Value> {
        if !needs_secret(id) {
            bail!("{} has no token here", name_of(id));
        }
        self.secrets().remove(id)?;
        self.db.set_integration(&IntegrationRow {
            id: id.into(),
            status: McpStatus::NotConfigured,
            detail: None,
            account: None,
            checked_at_ms: Some(now_ms()),
        })?;
        self.event("-", "integration_secret_removed", json!({"id": id}));
        Ok(self.integration_json(id))
    }

    /// Tests an integration now and records the outcome.
    pub fn integration_test(&self, id: &str) -> Result<Value> {
        if let Some(tool) = design_tool(id) {
            let ids: Vec<(Option<String>, String)> = self
                .db
                .mcp_servers_all()?
                .into_iter()
                .filter(|(_, s)| s.design_tool == Some(tool))
                .map(|(scope, s)| (scope.project().map(str::to_string), s.id))
                .collect();
            for (project, sid) in ids {
                self.mcp_server_test(&sid, project.as_deref())?;
            }
            return Ok(self.integration_json(id));
        }
        let (status, detail, account) = match id {
            "github" => match self.cfg.net.gh_auth_status() {
                Ok(text) => {
                    let acct = gh_account(&text);
                    let detail = acct.as_ref().map(|a| format!("logged in as {a}"));
                    (McpStatus::Connected, detail.or(Some("gh is logged in".into())), acct)
                }
                Err(e) => {
                    let t = format!("{e:#}");
                    let st = if t.contains("not logged") || t.contains("auth login") {
                        McpStatus::NeedsAuth
                    } else {
                        McpStatus::Error
                    };
                    (st, Some(t.lines().last().unwrap_or("").trim().chars().take(200).collect()), None)
                }
            },
            "linear" | "notion" => {
                let Some(token) = self.secrets().get(id)? else {
                    return Ok(self.integration_json(id));
                };
                let req = if id == "linear" {
                    HttpRequest {
                        method: "POST",
                        url: LINEAR_URL.into(),
                        headers: vec![
                            ("Authorization".into(), token.expose().into()),
                            ("Content-Type".into(), "application/json".into()),
                        ],
                        body: Some(json!({"query": "{ viewer { name } organization { name } }"}).to_string()),
                        timeout: DEADLINE,
                    }
                } else {
                    HttpRequest {
                        method: "GET",
                        url: NOTION_URL.into(),
                        headers: vec![
                            ("Authorization".into(), format!("Bearer {}", token.expose())),
                            ("Notion-Version".into(), NOTION_VERSION.into()),
                        ],
                        body: None,
                        timeout: DEADLINE,
                    }
                };
                match self.cfg.net.http(&req) {
                    Ok(r) if id == "linear" => linear_outcome(&r),
                    Ok(r) => notion_outcome(&r),
                    Err(e) => (McpStatus::Error, Some(format!("{}: {e:#}", host_of(&req.url))), None),
                }
            }
            other => return Err(anyhow!("no integration {other}")),
        };
        self.db.set_integration(&IntegrationRow {
            id: id.into(),
            status,
            detail,
            account,
            checked_at_ms: Some(now_ms()),
        })?;
        Ok(self.integration_json(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::tests::test_runtime_net;
    use orch_team::Scope;
    use orch_team::mcp::{McpKind, McpServer};

    #[test]
    fn gh_accounts() {
        let new = "github.com\n  ✓ Logged in to github.com account GuillaumeOj (keyring)\n  - Active account: true\n";
        assert_eq!(gh_account(new).as_deref(), Some("GuillaumeOj"));
        let old = "github.com\n  ✓ Logged in to github.com as octo-cat (oauth_token)\n";
        assert_eq!(gh_account(old).as_deref(), Some("octo-cat"));
        assert_eq!(gh_account("You are not logged into any GitHub hosts."), None);
    }

    #[test]
    fn linear_and_notion_answers() {
        let r = |status, body: &str| HttpResponse { status, body: body.into(), ..Default::default() };
        // What api.linear.app answers to a bad key (seen live).
        let bad = r#"{"errors":[{"message":"Authentication required, not authenticated","extensions":{"type":"authentication error","code":"AUTHENTICATION_ERROR","statusCode":401}}]}"#;
        assert_eq!(linear_outcome(&r(401, bad)).0, McpStatus::NeedsAuth);
        assert_eq!(linear_outcome(&r(400, bad)).0, McpStatus::NeedsAuth);
        let ok = r#"{"data":{"viewer":{"name":"G"},"organization":{"name":"Acme"}}}"#;
        assert_eq!(
            linear_outcome(&r(200, ok)),
            (McpStatus::Connected, Some("logged in as G".into()), Some("Acme".into()))
        );
        assert_eq!(linear_outcome(&r(500, "")).0, McpStatus::Error);
        let bad = r#"{"object":"error","status":401,"code":"unauthorized","message":"API token is invalid."}"#;
        assert_eq!(notion_outcome(&r(401, bad)).0, McpStatus::NeedsAuth);
        let ok = r#"{"object":"user","name":"Orch","type":"bot","bot":{"workspace_name":"Acme"}}"#;
        assert_eq!(notion_outcome(&r(200, ok)).2.as_deref(), Some("Acme"));
    }

    #[test]
    fn tokens_go_to_the_keychain_and_never_back() {
        let (_d, rt, net) = test_runtime_net();
        let v = rt.integrations_json();
        let ids: Vec<&str> = v["items"].as_array().unwrap().iter().map(|i| i["id"].as_str().unwrap()).collect();
        assert_eq!(ids, IDS);
        let linear = rt.integration_json("linear");
        assert_eq!((linear["status"].as_str(), linear["needs_secret"].as_bool()), (Some("not_configured"), Some(true)));
        assert_eq!(rt.integration_json("github")["needs_secret"], false);
        // Penpot: the seeded connector entry.
        assert_eq!(rt.integration_json("penpot")["status"], "unknown");
        assert_eq!(rt.integration_json("figma")["status"], "not_configured");

        net.route(LINEAR_URL, 200, &[], r#"{"data":{"viewer":{"name":"G"},"organization":{"name":"Acme"}}}"#);
        let v = rt.integration_secret_set("linear", Secret::new(" lin_api_SECRET ")).unwrap();
        assert_eq!(v["status"], "connected");
        assert_eq!(v["account"], "Acme");
        assert_eq!(v["has_secret"], true);
        assert!(!v.to_string().contains("lin_api_SECRET"));
        // Sent to Linear in a header only (curl reads headers from stdin).
        let seen = net.seen.lock().unwrap().last().cloned().unwrap();
        assert_eq!(seen.headers[0], ("Authorization".into(), "lin_api_SECRET".into()));
        assert!(!seen.body.unwrap_or_default().contains("lin_api_SECRET") && !seen.url.contains("lin_api_SECRET"));
        assert_eq!(rt.secrets().get("linear").unwrap().unwrap().expose(), "lin_api_SECRET");
        // Nowhere in the database: not in the integrations table, not in events.
        rt.db.flush().unwrap();
        let dump: String = rt
            .db
            .read(|c| {
                let mut st = c.prepare(
                    "SELECT COALESCE(detail,'') || COALESCE(account,'') FROM integrations
                     UNION ALL SELECT detail FROM events UNION ALL SELECT value FROM kv",
                )?;
                let v = st.query_map([], |r| r.get::<_, String>(0))?.flatten().collect::<Vec<_>>();
                Ok(v.join("\n"))
            })
            .unwrap();
        assert!(!dump.contains("lin_api_SECRET"), "{dump}");

        // A revoked key → needs_auth.
        net.route(LINEAR_URL, 401, &[], r#"{"errors":[{"extensions":{"code":"AUTHENTICATION_ERROR"}}]}"#);
        assert_eq!(rt.integration_test("linear").unwrap()["status"], "needs_auth");
        let v = rt.integration_secret_remove("linear").unwrap();
        assert_eq!((v["status"].as_str(), v["has_secret"].as_bool()), (Some("not_configured"), Some(false)));
        assert!(rt.secrets().get("linear").unwrap().is_none());
        assert!(rt.integration_secret_set("github", Secret::new("x")).is_err());
        assert!(rt.integration_secret_set("notion", Secret::new("  ")).is_err());

        // Unreachable → error, with the host.
        rt.integration_secret_set("notion", Secret::new("ntn_x")).unwrap();
        let v = rt.integration_test("notion").unwrap();
        assert_eq!(v["status"], "error");
        assert!(v["detail"].as_str().unwrap().contains("api.notion.com"));
        // An MCP env account: stored, not tested.
        assert_eq!(rt.integration_secret_set("figma-token", Secret::new("f")).unwrap()["has_secret"], true);
    }

    #[test]
    fn github_and_design_tools() {
        let (_d, rt, net) = test_runtime_net();
        *net.gh.lock().unwrap() = Some(Ok("  ✓ Logged in to github.com account Octo (keyring)".into()));
        let v = rt.integration_test("github").unwrap();
        assert_eq!((v["status"].as_str(), v["account"].as_str()), (Some("connected"), Some("Octo")));
        *net.gh.lock().unwrap() =
            Some(Err("You are not logged into any GitHub hosts. To log in, run: gh auth login".into()));
        assert_eq!(rt.integration_test("github").unwrap()["status"], "needs_auth");

        let mut f = McpServer::new("figma", "Figma", McpKind::Http);
        f.url = Some("https://figma.example/mcp".into());
        f.design_tool = Some(DesignTool::Figma);
        rt.mcp_server_save(f, &Scope::Global).unwrap();
        net.route("https://figma.example/mcp", 200, &[], "{}");
        let v = rt.integration_test("figma").unwrap();
        assert_eq!((v["status"].as_str(), v["detail"].as_str()), (Some("connected"), Some("MCP: Figma")));
        assert!(v["checked_at_ms"].as_u64().is_some());
    }
}
