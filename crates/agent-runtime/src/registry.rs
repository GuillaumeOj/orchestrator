//! The MCP server registry at runtime (FR-1202, FR-601, FR-604): a role's
//! `mcp_refs` resolved at each spawn (project override > global) with the
//! opted-in Keychain env values, the auth probe of each server, and the one
//! Inbox alert per server that needs the user to sign in again.
//!
//! What the probe can know (verified against Claude Code 2.1.311, see the
//! README "MCP auth status"): an OAuth server answers 401 to the core
//! whether or not the user signed in, because Claude Code keeps the token.
//! So a 401 that announces OAuth is `unknown`, and the agents' own tool
//! calls tell the rest: a call to `mcp__<server>__authenticate` (what Claude
//! Code exposes instead of the tools of a server that needs a sign-in) or an
//! auth-looking failure marks it `needs_auth`; a successful call `connected`.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use orch_protocol::{HookEvent, HookPayload, now_ms};
use orch_secrets::{Secret, Secrets};
use orch_team::mcp::{McpKind, McpServer, McpStatus, Registry, UsedBy, keychain_account};
use orch_team::{RoleDef, Scope, Team};
use serde_json::{Map, Value, json};

use crate::net::{HttpRequest, Net, host_of};
use crate::role::Role;
use crate::runtime::Runtime;
use crate::store::{AgentRecord, InboxStatus};

/// FR-604: a stdio server must answer `initialize` and `tools/list` within this.
pub const STDIO_DEADLINE: Duration = Duration::from_secs(10);
/// An http/sse probe's deadline.
pub const HTTP_DEADLINE: Duration = Duration::from_secs(10);
/// Servers used by running agents are probed again this often.
pub const PROBE_EVERY_MS: u64 = 30 * 60 * 1000;
/// A server probed this recently is not probed again in the background (the
/// agents of a team spawn together); Test always probes.
pub const PROBE_FRESH_MS: u64 = 60 * 1000;
/// The Inbox alert payload's `alert`.
pub const NEEDS_AUTH_ALERT: &str = "mcp_needs_auth";

const MCP_PROTOCOL: &str = "2025-06-18";

fn initialize() -> Value {
    json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": MCP_PROTOCOL, "capabilities": {},
        "clientInfo": {"name": "orchestrator-probe", "version": env!("CARGO_PKG_VERSION")}}})
}

/// Does an error text look like an expired or missing sign-in?
pub fn looks_like_auth(text: &str) -> bool {
    let t = text.to_ascii_lowercase();
    [
        "401",
        "403",
        "unauthorized",
        "unauthorised",
        "forbidden",
        "authenticat",
        "not logged in",
        "invalid token",
        "invalid_token",
        "token expired",
        "expired token",
        "invalid_grant",
        "re-auth",
        "reauth",
        "sign in again",
        "log in again",
    ]
    .iter()
    .any(|k| t.contains(k))
}

/// `mcp__<server>__<tool>` → `(server, tool)`; `None` for other tools and the
/// orchestrator's own.
pub fn mcp_tool(name: &str) -> Option<(&str, &str)> {
    let rest = name.strip_prefix("mcp__")?;
    let (server, tool) = rest.split_once("__")?;
    (server != orch_team::compile::ORCH_SERVER).then_some((server, tool))
}

/// The tools Claude Code puts in place of a server's own when it needs a sign-in.
fn is_auth_tool(tool: &str) -> bool {
    tool == "authenticate" || tool == "complete_authentication"
}

/// Replaces the `keychain:<account>` env values of the servers in `allowed`;
/// any other `keychain:` value is refused (the entry's opt-in is off).
pub fn resolve_secret_env(servers: &mut Map<String, Value>, allowed: &[String], secrets: &dyn Secrets) -> Result<()> {
    for (name, entry) in servers.iter_mut() {
        let Some(env) = entry.get_mut("env").and_then(Value::as_object_mut) else { continue };
        for (k, v) in env.iter_mut() {
            let Some(acct) = v.as_str().and_then(keychain_account) else { continue };
            if !allowed.iter().any(|a| a == name) {
                bail!("mcp server `{name}`: {k} reads a Keychain secret, which is not allowed for this server");
            }
            let s = secrets
                .get(&orch_secrets::mcp_account(acct))?
                .ok_or_else(|| anyhow!("mcp server `{name}`: no Keychain secret `{acct}`"))?;
            *v = Value::String(s.expose().to_string());
        }
    }
    Ok(())
}

/// The check of [`resolve_secret_env`] without reading the Keychain: a
/// `keychain:` env value is allowed only on the servers in `allowed`.
pub fn check_secret_env(servers: &Map<String, Value>, allowed: &[String]) -> Result<()> {
    for (name, entry) in servers {
        let Some(env) = entry.get("env").and_then(Value::as_object) else { continue };
        for (k, v) in env {
            if v.as_str().and_then(keychain_account).is_some() && !allowed.iter().any(|a| a == name) {
                bail!("mcp server `{name}`: {k} reads a Keychain secret, which is not allowed for this server");
            }
        }
    }
    Ok(())
}

/// The env of a stdio probe: the entry's, its Keychain values resolved when allowed.
fn probe_env(s: &McpServer, secrets: &dyn Secrets) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    for (k, v) in &s.env {
        match keychain_account(v) {
            Some(_) if !s.allow_secret_env => bail!("{k} reads a Keychain secret without the opt-in"),
            Some(acct) => {
                let sec = secrets
                    .get(&orch_secrets::mcp_account(acct))?
                    .ok_or_else(|| anyhow!("no Keychain secret `{acct}`"))?;
                out.push((k.clone(), sec.expose().to_string()));
            }
            None => out.push((k.clone(), v.clone())),
        }
    }
    Ok(out)
}

/// FR-604: one server's status, now.
pub fn probe(s: &McpServer, secrets: &dyn Secrets, net: &dyn Net) -> (McpStatus, Option<String>) {
    match s.kind {
        McpKind::Connector => (
            McpStatus::Unknown,
            Some(
                "claude.ai connector: loaded by Claude Code from your login; checked on the agents' tool calls".into(),
            ),
        ),
        McpKind::Http | McpKind::Sse => probe_http(s, net),
        McpKind::Stdio => match probe_env(s, secrets) {
            Ok(env) => probe_stdio(s, &env, STDIO_DEADLINE),
            Err(e) => (McpStatus::Error, Some(format!("{e:#}"))),
        },
    }
}

fn probe_http(s: &McpServer, net: &dyn Net) -> (McpStatus, Option<String>) {
    let url = s.url.clone().unwrap_or_default();
    let host = host_of(&url).to_string();
    let req = if s.kind == McpKind::Sse {
        HttpRequest {
            method: "GET",
            url,
            headers: vec![("Accept".into(), "text/event-stream".into())],
            body: None,
            timeout: Duration::from_secs(5),
            max_body: crate::net::MAX_BODY,
        }
    } else {
        HttpRequest {
            method: "POST",
            url,
            headers: vec![
                ("Content-Type".into(), "application/json".into()),
                ("Accept".into(), "application/json, text/event-stream".into()),
            ],
            body: Some(initialize().to_string()),
            timeout: HTTP_DEADLINE,
            max_body: crate::net::MAX_BODY,
        }
    };
    match net.http(&req) {
        Err(e) => (McpStatus::Error, Some(format!("{host}: {e:#}"))),
        Ok(r) => http_status(&r, &host),
    }
}

/// Maps an `initialize` (or SSE `GET`) answer to a status.
pub fn http_status(r: &crate::net::HttpResponse, host: &str) -> (McpStatus, Option<String>) {
    match r.status {
        200..=299 => (McpStatus::Connected, Some(format!("{} from {host}", r.status))),
        401 | 403 => {
            let www = r.header("www-authenticate").unwrap_or("");
            if www.contains("resource_metadata") || www.to_ascii_lowercase().contains("oauth") {
                // The app never holds an OAuth token (FR-1206): Claude Code does.
                (
                    McpStatus::Unknown,
                    Some(format!(
                        "OAuth server ({} from {host} without a token): Claude Code keeps the sign-in; checked on the \
                         agents' tool calls",
                        r.status
                    )),
                )
            } else {
                (McpStatus::NeedsAuth, Some(format!("{} from {host}", r.status)))
            }
        }
        code => (McpStatus::Error, Some(format!("{code} from {host}"))),
    }
}

/// Spawns the server, `initialize` then `tools/list`, within `deadline`.
pub fn probe_stdio(s: &McpServer, env: &[(String, String)], deadline: Duration) -> (McpStatus, Option<String>) {
    let Some(cmd) = s.command.as_deref() else { return (McpStatus::Error, Some("no command".into())) };
    let mut c = Command::new(cmd);
    c.args(&s.args)
        .envs(env.iter().map(|(k, v)| (k, v)))
        .current_dir(crate::paths::home_dir())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = match c.spawn() {
        Ok(ch) => ch,
        Err(e) => return (McpStatus::Error, Some(format!("can't start {cmd}: {e}"))),
    };
    let (tx, rx) = mpsc::channel::<String>();
    let out = child.stdout.take().map(BufReader::new);
    std::thread::spawn(move || {
        if let Some(out) = out {
            for line in out.lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        }
    });
    // The last stderr line, kept as it comes: a process the server started
    // (`npx`, `uvx`) may outlive the kill below and hold the pipe open, so
    // the reader is never joined, only waited for a moment.
    let err_pipe = child.stderr.take();
    let tail = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let (err_done, err_done_rx) = mpsc::channel::<()>();
    let tail_w = tail.clone();
    std::thread::spawn(move || {
        if let Some(e) = err_pipe {
            for line in BufReader::new(e).lines().map_while(Result::ok) {
                *tail_w.lock().unwrap_or_else(|e| e.into_inner()) = line;
            }
        }
        let _ = err_done.send(());
    });
    let mut stdin = child.stdin.take();
    let mut send =
        |v: Value| -> bool { stdin.as_mut().is_some_and(|i| writeln!(i, "{v}").and_then(|_| i.flush()).is_ok()) };
    let end = Instant::now() + deadline;
    let wait_for = |id: u64| -> Result<Value, String> {
        loop {
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(format!("no answer within {} s", deadline.as_secs()));
            }
            match rx.recv_timeout(left) {
                Ok(line) => {
                    let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
                    if v["id"] == json!(id) {
                        return Ok(v);
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    return Err(format!("no answer within {} s", deadline.as_secs()));
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => return Err("exited before answering".into()),
            }
        }
    };
    let result = (|| {
        if !send(initialize()) {
            return Err("closed its input".to_string());
        }
        let init = wait_for(1)?;
        if let Some(e) = init.get("error") {
            return Err(format!("initialize: {}", e["message"].as_str().unwrap_or("error")));
        }
        send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        send(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}));
        let tools = wait_for(2)?;
        if let Some(e) = tools.get("error") {
            return Err(format!("tools/list: {}", e["message"].as_str().unwrap_or("error")));
        }
        Ok(tools["result"]["tools"].as_array().map_or(0, Vec::len))
    })();
    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
    match result {
        Ok(n) => (McpStatus::Connected, Some(format!("{n} tool(s)"))),
        Err(e) => {
            let _ = err_done_rx.recv_timeout(Duration::from_secs(1));
            let tail = tail.lock().unwrap_or_else(|e| e.into_inner()).clone();
            let text = if tail.trim().is_empty() { e } else { format!("{e}: {}", tail.trim()) };
            let status = if looks_like_auth(&text) { McpStatus::NeedsAuth } else { McpStatus::Error };
            (status, Some(text.chars().take(300).collect()))
        }
    }
}

/// Where an entry seen from `project` lives.
fn scope_in(reg: &Registry, project: Option<&str>, id: &str) -> Scope {
    match project {
        Some(p) if reg.project.contains_key(id) => Scope::Project(p.into()),
        _ => Scope::Global,
    }
}

/// The role a team node uses, as the team's scope sees it.
fn role_for<'a>(roles: &'a [RoleDef], team: &Team, role_id: &str) -> Option<&'a RoleDef> {
    let p = team.scope.project();
    roles
        .iter()
        .find(|r| r.id == role_id && p.is_some() && r.scope.project() == p)
        .or_else(|| roles.iter().find(|r| r.id == role_id && r.scope == Scope::Global))
}

impl Runtime {
    pub fn secrets(&self) -> &dyn Secrets {
        self.cfg.secrets.as_ref()
    }

    /// An MCP env secret (`keychain:<account>` in a server's env), stored in
    /// the Keychain under `mcp:<account>`, apart from the integrations' tokens.
    /// The value is never logged or returned.
    pub fn mcp_secret_set(&self, account: &str, value: Secret) -> Result<()> {
        if !orch_secrets::valid_account(account) {
            bail!("invalid account {account:?} (a-z, 0-9, - _ ., up to 64)");
        }
        if value.expose().is_empty() {
            bail!("empty value");
        }
        self.secrets().set(&orch_secrets::mcp_account(account), &value)?;
        self.event("-", "mcp_secret_set", json!({"account": account}));
        Ok(())
    }

    pub fn mcp_secret_delete(&self, account: &str) -> Result<()> {
        if !orch_secrets::valid_account(account) {
            bail!("invalid account {account:?}");
        }
        self.secrets().remove(&orch_secrets::mcp_account(account))?;
        self.event("-", "mcp_secret_deleted", json!({"account": account}));
        Ok(())
    }

    /// Which MCP env secrets are stored (never their values); an invalid name is `false`.
    pub fn mcp_secrets_status(&self, accounts: &[String]) -> Value {
        let m: serde_json::Map<String, Value> = accounts
            .iter()
            .map(|a| {
                let has = orch_secrets::valid_account(a) && self.secrets().has(&orch_secrets::mcp_account(a));
                (a.clone(), json!(has))
            })
            .collect();
        Value::Object(m)
    }

    /// The project an agent works in.
    pub(crate) fn agent_project(&self, rec: &AgentRecord) -> Option<String> {
        rec.team
            .as_ref()
            .map(|t| t.project_id.clone())
            .or_else(|| self.db.project_by_path(&rec.repo).ok().flatten().map(|p| p.id))
    }

    /// A role with its registry refs resolved into `mcp_servers`. With
    /// `secrets`, the opted-in `keychain:` env values are read too (spawn
    /// only: the result is written to the agent's `mcp.json`, never stored).
    pub fn resolve_mcp(&self, role: &Role, project: Option<&str>, secrets: bool) -> Result<Role> {
        let mut out = role.clone();
        let mut allowed = vec![];
        if !role.mcp_refs.is_empty() {
            let reg = self.db.mcp_registry(project)?;
            let r = orch_team::mcp::resolve(&role.mcp_refs, role.strict_mcp, &reg).map_err(|e| anyhow!(e))?;
            out.mcp_servers.extend(r.servers);
            allowed = r.secret_env;
            if r.strict_off {
                out.strict_mcp = false;
            }
        }
        if secrets {
            resolve_secret_env(&mut out.mcp_servers, &allowed, self.secrets())?;
        } else {
            // Same refusal, without reading the Keychain.
            check_secret_env(&out.mcp_servers, &allowed)?;
        }
        Ok(out)
    }

    /// The registry for the view (api-contract §2): global entries and the
    /// project's overrides, each with the role nodes using it.
    pub fn mcp_registry_json(&self, project: Option<&str>) -> Result<Value> {
        let reg = self.db.mcp_registry(project)?;
        let (roles, teams) = (self.db.roles_all()?, self.db.teams_all()?);
        let overrides = self.project_overrides()?;
        let fill = |mut s: McpServer, scope: &Scope| {
            s.used_by = used_by(&overrides, &roles, &teams, &s.id, scope);
            s
        };
        let servers: Vec<McpServer> = reg.global.values().cloned().map(|s| fill(s, &Scope::Global)).collect();
        let overrides: Map<String, Value> = match project {
            Some(p) => reg
                .project
                .values()
                .cloned()
                .map(|s| (s.id.clone(), json!(fill(s, &Scope::Project(p.into())))))
                .collect(),
            None => Map::new(),
        };
        Ok(json!({"servers": servers, "project_overrides": overrides}))
    }

    /// One entry with its usage.
    fn server_json(&self, scope: &Scope, id: &str) -> Result<McpServer> {
        let mut s = self.db.mcp_server(scope, id)?.ok_or_else(|| anyhow!("no MCP server {id}"))?;
        s.used_by = used_by(&self.project_overrides()?, &self.db.roles_all()?, &self.db.teams_all()?, id, scope);
        Ok(s)
    }

    pub fn mcp_server_save(&self, server: McpServer, scope: &Scope) -> Result<McpServer> {
        server.validate().map_err(|e| anyhow!(e))?;
        if let Some(p) = scope.project()
            && self.db.project(p)?.is_none()
        {
            bail!("no project {p}");
        }
        self.db.save_mcp_server(scope, &server, now_ms())?;
        self.server_json(scope, &server.id)
    }

    /// Deletes an entry unless a role would lose it (a project override
    /// falls back to the global entry, so it can always go).
    pub fn mcp_server_delete(&self, id: &str, scope: &Scope) -> Result<Value> {
        if self.db.mcp_server(scope, id)?.is_none() {
            bail!("no MCP server {id}");
        }
        let falls_back = scope.project().is_some() && self.db.mcp_server(&Scope::Global, id)?.is_some();
        let used = used_by(&self.project_overrides()?, &self.db.roles_all()?, &self.db.teams_all()?, id, scope);
        if !falls_back && !used.is_empty() {
            return Ok(json!({"deleted": false, "used_by": used}));
        }
        self.db.delete_mcp_server(scope, id)?;
        self.close_needs_auth(id, scope.project(), "deleted");
        Ok(json!({"deleted": true, "used_by": []}))
    }

    /// Probes `id` as `project` sees it and records the outcome.
    pub fn mcp_server_test(&self, id: &str, project: Option<&str>) -> Result<McpServer> {
        let scope = self.probe_server(id, project)?;
        self.server_json(&scope, id)
    }

    /// FR-604: probes one server now and records its status; returns its scope.
    fn probe_server(&self, id: &str, project: Option<&str>) -> Result<Scope> {
        let reg = self.db.mcp_registry(project)?;
        let s = reg.get(id).ok_or_else(|| anyhow!("no MCP server {id}"))?.clone();
        let scope = scope_in(&reg, project, id);
        let (status, detail) = probe(&s, self.secrets(), self.cfg.net.as_ref());
        self.set_mcp_status(&scope, &s, status, detail.as_deref())?;
        Ok(scope)
    }

    /// Every project override, by (project, id), for [`used_by`].
    fn project_overrides(&self) -> Result<Overrides> {
        Ok(self
            .db
            .mcp_servers_all()?
            .into_iter()
            .filter_map(|(scope, s)| Some((scope.project()?.to_string(), s.id)))
            .collect())
    }

    /// Probes the servers of `refs` on a thread (at spawn, and every 30 min
    /// while used). Connectors are skipped (nothing to probe), and so are
    /// servers probed less than [`PROBE_FRESH_MS`] ago or being probed now
    /// (several agents of a team spawn together).
    pub(crate) fn probe_in_background(&self, project: Option<String>, refs: Vec<String>) {
        let Some(me) = self.me.upgrade() else { return };
        let Ok(reg) = self.db.mcp_registry(project.as_deref()) else { return };
        let now = now_ms();
        let todo: Vec<String> = {
            let mut busy = self.probing.lock().unwrap_or_else(|e| e.into_inner());
            refs.into_iter()
                .filter(|id| {
                    reg.get(id).is_some_and(|s| {
                        s.kind != McpKind::Connector
                            && s.checked_at_ms.is_none_or(|t| now.saturating_sub(t) >= PROBE_FRESH_MS)
                    })
                })
                .filter(|id| busy.insert((project.clone(), id.clone())))
                .collect()
        };
        if todo.is_empty() {
            return;
        }
        std::thread::spawn(move || {
            for id in todo {
                let _ = me.probe_server(&id, project.as_deref());
                me.probing.lock().unwrap_or_else(|e| e.into_inner()).remove(&(project.clone(), id));
            }
        });
    }

    /// FR-604 every 30 min: the servers the running agents use.
    pub(crate) fn probe_used_servers(&self) {
        let mut by_project: std::collections::BTreeMap<Option<String>, Vec<String>> = Default::default();
        for id in self.live_ids() {
            let Some(rec) = self.store.read(|s| s.agents.get(&id).cloned()) else { continue };
            if rec.role.mcp_refs.is_empty() {
                continue;
            }
            let v = by_project.entry(self.agent_project(&rec)).or_default();
            v.extend(rec.role.mcp_refs.iter().cloned());
        }
        for (p, mut refs) in by_project {
            refs.sort();
            refs.dedup();
            self.probe_in_background(p, refs);
        }
    }

    /// Records a status; `needs_auth` raises the server's one Inbox alert,
    /// `connected` closes it.
    ///
    /// A probe that can't tell (`unknown`: an OAuth server, a connector)
    /// never replaces what the agents' calls showed (`connected`,
    /// `needs_auth`): only the detail and the check time change.
    pub fn set_mcp_status(&self, scope: &Scope, s: &McpServer, status: McpStatus, detail: Option<&str>) -> Result<()> {
        let observed = matches!(s.status, McpStatus::Connected | McpStatus::NeedsAuth);
        let kept = if status == McpStatus::Unknown && observed { s.status } else { status };
        self.db.set_mcp_status(scope, &s.id, kept, detail, now_ms())?;
        if kept != s.status {
            self.event("-", "mcp_status", json!({"server": s.id, "status": kept, "detail": detail}));
        }
        let project = scope.project();
        match status {
            McpStatus::NeedsAuth => self.raise_needs_auth(s, project, detail),
            McpStatus::Connected => self.close_needs_auth(&s.id, project, "connected"),
            _ => {}
        }
        Ok(())
    }

    /// Pending alerts of the entry `id` in `project` (`None`: the global one).
    fn pending_needs_auth(&self, id: &str, project: Option<&str>) -> Vec<u64> {
        self.store.read(|st| {
            st.inbox
                .iter()
                .filter(|i| i.pending() && i.payload["alert"] == NEEDS_AUTH_ALERT && i.payload["server"] == id)
                .filter(|i| i.payload["project"].as_str() == project)
                .map(|i| i.id)
                .collect()
        })
    }

    fn raise_needs_auth(&self, s: &McpServer, project: Option<&str>, detail: Option<&str>) {
        if !self.pending_needs_auth(&s.id, project).is_empty() {
            return; // one per server
        }
        let mut roles: Vec<String> = self
            .db
            .roles_all()
            .unwrap_or_default()
            .into_iter()
            .filter(|r| r.mcp_refs.contains(&s.id))
            .map(|r| r.id)
            .collect();
        // Listed by scope first: the same role id of two scopes is not adjacent.
        roles.sort();
        roles.dedup();
        let body = format!(
            "MCP server {} needs you to sign in again{}",
            s.name,
            detail.map(|d| format!(": {d}")).unwrap_or_default()
        );
        // `project`: whose entry it is (null: the global one), for the reconnect target.
        let payload =
            json!({"alert": NEEDS_AUTH_ALERT, "server": s.id, "name": s.name, "roles": roles, "project": project});
        let _ = self.add_item(None, "alert", body, payload, 0);
    }

    fn close_needs_auth(&self, id: &str, project: Option<&str>, why: &str) {
        let open = self.pending_needs_auth(id, project);
        if open.is_empty() {
            return;
        }
        let _ = self.store.write(|st| {
            for i in open {
                let _ = st.close_inbox(i, InboxStatus::Dismissed, Some(why.into()));
            }
        });
    }

    /// FR-604 at runtime: an agent's call to a registry server's tool.
    pub(crate) fn observe_mcp_call(&self, rec: &AgentRecord, event: &HookEvent, p: &HookPayload) {
        let Some((server, tool)) = p.tool_name.as_deref().and_then(mcp_tool) else { return };
        let status = match event {
            HookEvent::PreToolUse | HookEvent::PostToolUse | HookEvent::PostToolUseFailure if is_auth_tool(tool) => {
                McpStatus::NeedsAuth
            }
            HookEvent::PostToolUseFailure if p.error.as_deref().is_some_and(looks_like_auth) => McpStatus::NeedsAuth,
            HookEvent::PostToolUse => McpStatus::Connected,
            _ => return,
        };
        let project = self.agent_project(rec);
        let Ok(reg) = self.db.mcp_registry(project.as_deref()) else { return };
        let Some(s) = reg.effective().into_iter().find(|s| s.owns_tool_server(server)).cloned() else { return };
        if s.status == status && status == McpStatus::Connected {
            return; // the common case: nothing to write
        }
        let detail = match status {
            McpStatus::NeedsAuth if is_auth_tool(tool) => {
                format!("{} called {tool}: Claude Code has no sign-in for it", rec.id)
            }
            McpStatus::NeedsAuth => {
                format!("{}: {}", rec.id, p.error.as_deref().unwrap_or("").chars().take(200).collect::<String>())
            }
            _ => format!("{} used {tool}", rec.id),
        };
        let scope = scope_in(&reg, project.as_deref(), &s.id);
        let _ = self.set_mcp_status(&scope, &s, status, Some(&detail));
    }
}

/// The role nodes that resolve `id` to the entry in `scope`: a global entry
/// is used where no project override shadows it.
/// `(project, id)` of every project override of a registry entry.
type Overrides = std::collections::HashSet<(String, String)>;

fn used_by(overrides: &Overrides, roles: &[RoleDef], teams: &[Team], id: &str, scope: &Scope) -> Vec<UsedBy> {
    let overridden = |p: Option<&str>| p.is_some_and(|p| overrides.contains(&(p.to_string(), id.to_string())));
    let reaches = |p: Option<&str>| match scope {
        Scope::Global => !overridden(p),
        Scope::Project(sp) => p == Some(sp.as_str()),
    };
    let mut out = Vec::new();
    let mut in_team = std::collections::BTreeSet::new();
    for t in teams {
        for n in &t.nodes {
            let Some(r) = role_for(roles, t, &n.role_id) else { continue };
            if r.mcp_refs.iter().any(|x| x == id) {
                in_team.insert((r.scope.project().map(str::to_string), r.id.clone()));
                if reaches(t.scope.project()) {
                    out.push(UsedBy { team: t.id.clone(), node: n.id.clone(), role: r.id.clone() });
                }
            }
        }
    }
    // Roles in no team still count (deleting the entry would break them).
    for r in roles {
        let key = (r.scope.project().map(str::to_string), r.id.clone());
        if r.mcp_refs.iter().any(|x| x == id) && !in_team.contains(&key) && reaches(r.scope.project()) {
            out.push(UsedBy { team: String::new(), node: String::new(), role: r.id.clone() });
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::net::fake::FakeNet;
    use crate::runtime::tests::{hook, test_runtime};
    use orch_secrets::{MemorySecrets, Secret};
    use std::sync::Arc;

    fn http(id: &str, url: &str) -> McpServer {
        let mut s = McpServer::new(id, id, McpKind::Http);
        s.url = Some(url.into());
        s
    }

    #[test]
    fn auth_texts_and_tool_names() {
        assert!(looks_like_auth("HTTP 401 Unauthorized"));
        assert!(looks_like_auth("Error: token expired, please re-authenticate"));
        assert!(!looks_like_auth("file not found"));
        assert_eq!(
            mcp_tool("mcp__claude_ai_Penpot_Perso__execute_code"),
            Some(("claude_ai_Penpot_Perso", "execute_code"))
        );
        assert_eq!(mcp_tool("mcp__orch__commit"), None);
        assert_eq!(mcp_tool("Bash"), None);
    }

    #[test]
    fn http_answers_map_to_statuses() {
        let r = |status, www: &str| crate::net::HttpResponse {
            status,
            headers: if www.is_empty() { vec![] } else { vec![("www-authenticate".into(), www.into())] },
            ..Default::default()
        };
        assert_eq!(http_status(&r(200, ""), "h").0, McpStatus::Connected);
        assert_eq!(http_status(&r(401, ""), "h").0, McpStatus::NeedsAuth);
        assert_eq!(http_status(&r(403, "Basic"), "h").0, McpStatus::NeedsAuth);
        // What mcp.linear.app, mcp.notion.com and mcp.figma.com answer to the core.
        let oauth = r(401, "Bearer realm=\"OAuth\", resource_metadata=\"https://mcp.linear.app/.well-known/x\"");
        assert_eq!(http_status(&oauth, "h").0, McpStatus::Unknown);
        assert_eq!(http_status(&r(404, ""), "h").0, McpStatus::Error);
    }

    /// A stdio MCP server in a few lines of shell.
    fn fake_stdio(script: &str) -> McpServer {
        let mut s = McpServer::new("local", "Local", McpKind::Stdio);
        s.command = Some("/bin/sh".into());
        s.args = vec!["-c".into(), script.into()];
        s
    }

    #[test]
    fn stdio_probe_initialize_and_tools_list() {
        let ok = fake_stdio(
            r#"read l; echo '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18","capabilities":{},"serverInfo":{"name":"x","version":"1"}}}'
               read l; read l; echo '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"a"},{"name":"b"}]}}'; sleep 5"#,
        );
        assert_eq!(probe_stdio(&ok, &[], STDIO_DEADLINE), (McpStatus::Connected, Some("2 tool(s)".into())));
        let silent = fake_stdio("sleep 5");
        let (st, d) = probe_stdio(&silent, &[], Duration::from_millis(300));
        assert_eq!(st, McpStatus::Error);
        assert!(d.unwrap().contains("no answer"));
        let auth = fake_stdio("echo 'error: 401 Unauthorized, run login' >&2; exit 1");
        assert_eq!(probe_stdio(&auth, &[], STDIO_DEADLINE).0, McpStatus::NeedsAuth);
        let missing = McpServer { command: Some("/nonexistent/mcp".into()), ..fake_stdio("") };
        assert_eq!(probe_stdio(&missing, &[], STDIO_DEADLINE).0, McpStatus::Error);
        // The env reaches the server (a Keychain value only with the opt-in).
        let mut env = fake_stdio(
            r#"read l; [ "$TOKEN" = s3cret ] && echo '{"jsonrpc":"2.0","id":1,"result":{}}'; read l; read l; echo '{"jsonrpc":"2.0","id":2,"result":{"tools":[]}}'"#,
        );
        env.env.insert("TOKEN".into(), "keychain:tok".into());
        let secrets = MemorySecrets::with(&[("mcp:tok", "s3cret"), ("tok", "integration")]);
        let net = FakeNet::default();
        assert_eq!(probe(&env, &secrets, &net).0, McpStatus::Error, "no opt-in");
        env.allow_secret_env = true;
        assert_eq!(probe(&env, &secrets, &net), (McpStatus::Connected, Some("0 tool(s)".into())));
    }

    #[test]
    fn mcp_secrets_are_set_listed_and_deleted_apart_from_integration_tokens() {
        let (_d, rt) = crate::runtime::tests::test_runtime(true);
        rt.secrets().set("linear", &Secret::new("integration_token")).unwrap();
        rt.mcp_secret_set("linear", Secret::new("mcp_token")).unwrap();
        assert_eq!(rt.secrets().get("linear").unwrap().unwrap().expose(), "integration_token", "not overwritten");
        let st = rt.mcp_secrets_status(&["linear".into(), "other".into(), "bad:name".into()]);
        assert_eq!(st, json!({"linear": true, "other": false, "bad:name": false}));
        assert!(rt.mcp_secret_set("bad:name", Secret::new("x")).is_err());
        assert!(rt.mcp_secret_set("empty", Secret::new("")).is_err());
        rt.db.flush().unwrap();
        let events = rt.db.events(0, None, Some("mcp_secret"), 50).unwrap();
        assert_eq!(events.len(), 1);
        assert!(!format!("{events:?}").contains("mcp_token"), "the value is never logged");
        rt.mcp_secret_delete("linear").unwrap();
        assert_eq!(rt.mcp_secrets_status(&["linear".into()]), json!({"linear": false}));
        assert!(rt.secrets().get("linear").unwrap().is_some(), "the integration's token stays");
    }

    #[test]
    fn secret_env_is_resolved_only_where_allowed() {
        // `keychain:linear` reads the MCP namespace, never the Linear integration's token.
        let secrets = MemorySecrets::with(&[("mcp:linear", "lin_123"), ("linear", "integration_token")]);
        let mut servers: Map<String, Value> = serde_json::from_value(json!({
            "a": {"type": "stdio", "command": "x", "env": {"K": "keychain:linear", "P": "plain"}},
            "b": {"type": "stdio", "command": "y"},
        }))
        .unwrap();
        resolve_secret_env(&mut servers, &["a".into()], &secrets).unwrap();
        assert_eq!(servers["a"]["env"], json!({"K": "lin_123", "P": "plain"}));
        let mut raw: Map<String, Value> =
            serde_json::from_value(json!({"c": {"type": "stdio", "command": "x", "env": {"K": "keychain:linear"}}}))
                .unwrap();
        let e = resolve_secret_env(&mut raw, &[], &secrets).unwrap_err().to_string();
        assert!(e.contains("not allowed") && !e.contains("lin_123"), "{e}");
        assert!(resolve_secret_env(&mut servers.clone(), &["a".into()], &MemorySecrets::default()).is_ok());
        let mut missing = raw.clone();
        assert!(resolve_secret_env(&mut missing, &["c".into()], &MemorySecrets::default()).is_err());
    }

    #[test]
    fn registry_save_test_delete_and_one_alert_per_server() {
        let (_d, rt, fake) = test_runtime_net();
        fake.route("https://figma.example/mcp", 401, &[], "");
        let s = rt.mcp_server_save(http("figma", "https://figma.example/mcp"), &Scope::Global).unwrap();
        assert_eq!(s.status, McpStatus::Unknown);
        assert!(rt.mcp_server_save(http("Bad Id", "https://x"), &Scope::Global).is_err());

        // A role uses it through a team: deletion is refused with the list.
        let mut role = RoleDef::new("ui", "UI");
        role.mcp_refs = vec!["figma".into()];
        rt.db.upsert_role(&role, 1).unwrap();
        let (_, mut team) = orch_team::presets::design_sprint();
        team.id = "design".into();
        rt.db.save_team(&team, &[role.clone()], 1).unwrap();
        let v = rt.mcp_registry_json(None).unwrap();
        let figma = v["servers"].as_array().unwrap().iter().find(|s| s["id"] == "figma").unwrap().clone();
        assert_eq!(figma["used_by"][0]["role"], "ui");
        assert_eq!(figma["used_by"][0]["team"], "design");
        let del = rt.mcp_server_delete("figma", &Scope::Global).unwrap();
        assert_eq!(del["deleted"], false);
        assert_eq!(del["used_by"][0]["node"], figma["used_by"][0]["node"]);

        // 401 (no OAuth) → needs_auth and exactly one Inbox alert.
        let t = rt.mcp_server_test("figma", None).unwrap();
        assert_eq!(t.status, McpStatus::NeedsAuth);
        rt.mcp_server_test("figma", None).unwrap();
        let alerts: Vec<_> = rt.inbox().into_iter().filter(|i| i.payload["alert"] == NEEDS_AUTH_ALERT).collect();
        assert_eq!(alerts.len(), 1, "deduplicated");
        assert_eq!(
            alerts[0].payload,
            json!({"alert": "mcp_needs_auth", "server": "figma", "name": "figma", "roles": ["ui"], "project": null})
        );
        // Connected again → the alert is closed.
        fake.route("https://figma.example/mcp", 200, &[], "{}");
        assert_eq!(rt.mcp_server_test("figma", None).unwrap().status, McpStatus::Connected);
        assert!(rt.inbox().iter().all(|i| i.payload["alert"] != NEEDS_AUTH_ALERT));
        // The seeded connector is never probed over the network.
        assert_eq!(rt.mcp_server_test("penpot", None).unwrap().status, McpStatus::Unknown);
        assert_eq!(fake.seen.lock().unwrap().len(), 3, "three figma probes, none for penpot");
    }

    #[test]
    fn hooks_on_mcp_tools_mark_the_server() {
        let (_d, rt) = test_runtime(true);
        let penpot = || rt.db.mcp_server(&Scope::Global, "penpot").unwrap().unwrap();
        let ev = |event: &str, tool: &str, error: Option<&str>| {
            rt.on_hook(
                &hook(event, json!({"tool_name": tool, "tool_use_id": "t1", "tool_input": {}, "error": error})),
                None,
            );
        };
        let pending = || rt.inbox().iter().filter(|i| i.payload["alert"] == NEEDS_AUTH_ALERT).count();
        // What Claude Code exposes for a connector that needs a sign-in.
        ev("PreToolUse", "mcp__claude_ai_Penpot_Perso__authenticate", None);
        assert_eq!(penpot().status, McpStatus::NeedsAuth);
        assert_eq!(pending(), 1);
        ev("PostToolUse", "mcp__claude_ai_Penpot_Perso__execute_code", None);
        assert_eq!(penpot().status, McpStatus::Connected);
        assert_eq!(pending(), 0, "closed when connected again");
        ev("PostToolUseFailure", "mcp__claude_ai_Penpot_Perso__execute_code", Some("boom: bad shape"));
        assert_eq!(penpot().status, McpStatus::Connected, "not an auth error");
        ev("PostToolUseFailure", "mcp__claude_ai_Penpot_Perso__execute_code", Some("HTTP 401: token expired"));
        assert_eq!(penpot().status, McpStatus::NeedsAuth);
        ev("PostToolUseFailure", "mcp__claude_ai_Penpot_Perso__execute_code", Some("401 again"));
        ev("PostToolUseFailure", "mcp__unknown__x", Some("401"));
        assert_eq!(pending(), 1, "one per server, none for a server outside the registry");
    }

    #[test]
    fn a_probe_that_cant_tell_keeps_what_the_agents_showed() {
        let (_d, rt, fake) = test_runtime_net();
        let status = |id: &str| rt.db.mcp_server(&Scope::Global, id).unwrap().unwrap();
        let ev = |event: &str, tool: &str| {
            rt.on_hook(&hook(event, json!({"tool_name": tool, "tool_use_id": "t1", "tool_input": {}})), None);
        };
        // A connector seen working stays connected through probes (spawn, 30 min).
        ev("PostToolUse", "mcp__claude_ai_Penpot__export_shape");
        assert_eq!(status("penpot").status, McpStatus::Connected);
        let t = rt.mcp_server_test("penpot", None).unwrap();
        assert_eq!(t.status, McpStatus::Connected);
        assert!(t.checked_at_ms.is_some() && t.detail.unwrap().contains("connector"), "detail and time updated");

        // An OAuth server: needs_auth from a call; the probe's 401+OAuth keeps it, and
        // a dismissed alert is not raised again by the probe.
        let mut s = McpServer::new("linear", "Linear", McpKind::Http);
        s.url = Some("https://mcp.linear.example/mcp".into());
        rt.mcp_server_save(s, &Scope::Global).unwrap();
        fake.route(
            "https://mcp.linear.example/mcp",
            401,
            &[("www-authenticate", "Bearer resource_metadata=\"x\"")],
            "",
        );
        ev("PreToolUse", "mcp__linear__authenticate");
        assert_eq!(status("linear").status, McpStatus::NeedsAuth);
        let alert = rt.inbox().into_iter().find(|i| i.payload["server"] == "linear").unwrap();
        assert_eq!(alert.payload["project"], Value::Null, "the global entry");
        rt.store.write(|st| st.close_inbox(alert.id, InboxStatus::Dismissed, None).map(|_| ()).ok()).unwrap();
        assert_eq!(rt.mcp_server_test("linear", None).unwrap().status, McpStatus::NeedsAuth);
        assert!(rt.inbox().iter().all(|i| i.payload["server"] != "linear"), "the probe raised nothing");
        // A never-observed server takes the probe's answer.
        let mut s2 = McpServer::new("other", "Other", McpKind::Http);
        s2.url = Some("https://mcp.linear.example/mcp".into());
        rt.mcp_server_save(s2, &Scope::Global).unwrap();
        assert_eq!(rt.mcp_server_test("other", None).unwrap().status, McpStatus::Unknown);
    }

    #[test]
    fn a_project_override_alert_names_its_project() {
        let (_d, rt, fake) = test_runtime_net();
        let (_r, repo) = orch_git::fixtures::scratch_repo();
        let p = orch_git::project::register(&rt.db, &repo).unwrap();
        let mut s = McpServer::new("penpot", "Penpot (self-hosted)", McpKind::Http);
        s.url = Some("https://penpot.example/mcp".into());
        rt.mcp_server_save(s, &Scope::Project(p.id.clone())).unwrap();
        fake.route("https://penpot.example/mcp", 403, &[], "");
        assert_eq!(rt.mcp_server_test("penpot", Some(&p.id)).unwrap().status, McpStatus::NeedsAuth);
        let a = rt.inbox().into_iter().find(|i| i.payload["server"] == "penpot").unwrap();
        assert_eq!(a.payload["project"], json!(p.id));
        // The global entry's own alert is a different one; connected closes only its own.
        let g = rt.db.mcp_server(&Scope::Global, "penpot").unwrap().unwrap();
        rt.set_mcp_status(&Scope::Global, &g, McpStatus::NeedsAuth, None).unwrap();
        assert_eq!(rt.inbox().iter().filter(|i| i.payload["server"] == "penpot").count(), 2);
        rt.set_mcp_status(&Scope::Global, &g, McpStatus::Connected, None).unwrap();
        let left: Vec<_> = rt.inbox().into_iter().filter(|i| i.payload["server"] == "penpot").collect();
        assert_eq!((left.len(), left[0].payload["project"].clone()), (1, json!(p.id)));
    }

    /// A test runtime whose network is a [`FakeNet`].
    pub(crate) fn test_runtime_net() -> (tempfile::TempDir, Arc<Runtime>, Arc<FakeNet>) {
        let d = tempfile::Builder::new().prefix("orchrt").tempdir_in("/tmp").unwrap();
        let fake = Arc::new(FakeNet::default());
        let mut cfg = crate::runtime::tests::test_cfg(d.path(), true);
        cfg.net = fake.clone();
        let rt = Runtime::new(cfg).unwrap();
        rt.store.write(|s| s.agents.insert("a1".into(), crate::runtime::tests::test_agent(d.path()))).unwrap();
        (d, rt, fake)
    }

    #[test]
    fn teams_view_shows_mcp_issues() {
        let (_d, rt) = test_runtime(true);
        rt.install_presets().unwrap();
        let mut ui = rt.db.role(&Scope::Global, "ui").unwrap().unwrap();
        ui.mcp_refs = vec!["penpot".into(), "ghost".into()];
        ui.strict_mcp = true;
        rt.save_role(ui).unwrap();
        let v = rt.teams_json(None).unwrap();
        let ui = v["roles"].as_array().unwrap().iter().find(|r| r["id"] == "ui").unwrap();
        assert_eq!(ui["issues"], json!(["connector `penpot` with strict MCP", "mcp ref `ghost` not in the registry"]));
        assert_eq!(ui["mcp_refs"], json!(["penpot", "ghost"]));
        let sprint = v["teams"].as_array().unwrap().iter().find(|t| t["team"]["id"] == "design-sprint").unwrap();
        let mcp: Vec<&Value> =
            sprint["issues"].as_array().unwrap().iter().filter(|i| i["issue"]["kind"] == "mcp").collect();
        assert_eq!(mcp.len(), 1);
        assert_eq!(mcp[0]["text"], "ui: mcp ref `ghost` not in the registry");
        // A warning doesn't block a launch: not in `issues`.
        assert_eq!(sprint["warnings"][0]["text"], "ui: connector `penpot` with strict MCP");
        let lead = v["roles"].as_array().unwrap().iter().find(|r| r["id"] == "lead").unwrap();
        assert_eq!(lead["issues"], json!([]));
    }

    /// NFR-3: an agent's env, argv and per-agent files carry no integration
    /// token; an opted-in MCP secret lands only in that server's `mcp.json` entry.
    #[test]
    fn no_integration_token_reaches_an_agent() {
        let (d, rt) = test_runtime(true);
        rt.secrets().set("linear", &Secret::new("lin_api_LEAK")).unwrap();
        rt.secrets().set("notion", &Secret::new("ntn_LEAK")).unwrap();
        rt.mcp_secret_set("figma-pat", Secret::new("figd_OPTIN")).unwrap();
        let mut local = McpServer::new("figma-local", "Figma (local)", McpKind::Stdio);
        local.command = Some("figma-mcp".into());
        local.env.insert("FIGMA_TOKEN".into(), "keychain:figma-pat".into());
        local.allow_secret_env = true;
        rt.mcp_server_save(local, &Scope::Global).unwrap();
        // The starter UI role with registry refs, as a team node compiles it.
        let (mut roles, team) = orch_team::presets::design_sprint();
        roles.iter_mut().find(|r| r.id == "ui").unwrap().mcp_refs = vec!["penpot".into(), "figma-local".into()];
        let f = orch_team::FeatureCtx { slug: "f".into(), title: "F".into(), base: "main".into() };
        let node = orch_team::compile_node(&team, "ui", &roles, &f).unwrap();
        let role = rt.resolve_mcp(&Role::from_node(&node).unwrap(), None, true).unwrap();
        let mut cx = crate::compile::tests::ctx(crate::compile::SessionMode::New("u".into()));
        cx.agent_dir = d.path().join("agents/ui-1");
        let c = crate::compile::compile(&role, &cx);
        c.write().unwrap();
        let mut everything: Vec<String> = c.args.clone();
        everything.extend(c.env.iter().map(|(k, v)| format!("{k}={v}")));
        for f in std::fs::read_dir(&c.agent_dir).unwrap() {
            everything.push(std::fs::read_to_string(f.unwrap().path()).unwrap());
        }
        let all = everything.join("\n");
        assert!(!all.contains("lin_api_LEAK") && !all.contains("ntn_LEAK"), "an integration token reached the agent");
        // The opt-in: in mcp.json, under that server only; never in env or argv.
        assert_eq!(c.mcp["mcpServers"]["figma-local"]["env"]["FIGMA_TOKEN"], "figd_OPTIN");
        assert_eq!(all.matches("figd_OPTIN").count(), 1, "mcp.json only");
        assert!(!c.env.values().any(|v| v.contains("figd_OPTIN")) && !c.args.iter().any(|a| a.contains("figd_OPTIN")));
        assert!(!c.mcp["mcpServers"].as_object().unwrap().contains_key("penpot"));
        // The agent record keeps the refs, never a resolved value.
        let stored = serde_json::to_string(&Role::from_node(&node).unwrap()).unwrap();
        assert!(stored.contains("figma-local") && !stored.contains("figd_OPTIN"));
        // And the Keychain itself is out of the agent's sandbox.
        assert!(
            c.settings["sandbox"]["filesystem"]["denyRead"].as_array().unwrap().contains(&json!("~/Library/Keychains"))
        );
    }

    #[test]
    fn spawned_role_resolves_refs_and_refuses_secrets_without_opt_in() {
        let (_d, rt) = test_runtime(true);
        rt.mcp_secret_set("linear", Secret::new("lin_123")).unwrap();
        let mut s = McpServer::new("linear-mcp", "Linear MCP", McpKind::Stdio);
        s.command = Some("npx".into());
        s.env.insert("LINEAR_API_KEY".into(), "keychain:linear".into());
        rt.mcp_server_save(s.clone(), &Scope::Global).unwrap();
        let mut role = Role::solo();
        role.mcp_refs = vec!["linear-mcp".into(), "penpot".into()];
        let e = rt.resolve_mcp(&role, None, true).unwrap_err().to_string();
        assert!(e.contains("allow it") && !e.contains("lin_123"), "{e}");
        assert!(rt.resolve_mcp(&role, None, false).is_err(), "refused at launch too");
        s.allow_secret_env = true;
        rt.mcp_server_save(s, &Scope::Global).unwrap();
        let dry = rt.resolve_mcp(&role, None, false).unwrap();
        assert_eq!(dry.mcp_servers["linear-mcp"]["env"]["LINEAR_API_KEY"], "keychain:linear");
        let r = rt.resolve_mcp(&role, None, true).unwrap();
        assert_eq!(r.mcp_servers["linear-mcp"]["env"]["LINEAR_API_KEY"], "lin_123");
        assert!(!r.mcp_servers.contains_key("penpot"), "a connector comes from the user's login");
        role.strict_mcp = true;
        assert!(!rt.resolve_mcp(&role, None, false).unwrap().strict_mcp, "a connector turns strict MCP off");
    }
}
