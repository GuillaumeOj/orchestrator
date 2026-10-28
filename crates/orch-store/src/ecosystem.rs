//! P3 ecosystem: the MCP server registry (FR-1202) with each entry's last
//! auth probe (FR-604), and the integrations' status (FR-1203). No secret is
//! ever stored here (NFR-3): they live in the Keychain.

use anyhow::Result;
use orch_team::mcp::{McpServer, McpStatus, Registry};
use orch_team::{RoleDef, Scope, Team};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::db::Store;
use crate::util::{js, opt_u64, scope_key, scope_of};

/// An integration's last known status (api-contract §1, without what the
/// Keychain says).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IntegrationRow {
    pub id: String,
    pub status: McpStatus,
    pub detail: Option<String>,
    pub account: Option<String>,
    pub checked_at_ms: Option<u64>,
}

fn row_server(r: &rusqlite::Row) -> rusqlite::Result<(String, McpServer)> {
    let scope: String = r.get(0)?;
    let data: String = r.get(1)?;
    let mut s: McpServer = crate::util::from_js(&data)?;
    s.status = McpStatus::parse(&r.get::<_, String>(2)?);
    s.detail = r.get(3)?;
    s.checked_at_ms = opt_u64(r, 4)?;
    Ok((scope, s))
}

const SERVER_COLS: &str = "scope, data, status, detail, checked_at_ms";

impl Store {
    // ------------------------------------------------------------ MCP registry

    /// The registry as `project` sees it (global entries and that project's
    /// overrides); `None`: the global entries only.
    pub fn mcp_registry(&self, project: Option<&str>) -> Result<Registry> {
        let project = project.unwrap_or_default().to_string();
        self.read(move |c| {
            let mut st = c.prepare(&format!(
                "SELECT {SERVER_COLS} FROM mcp_registry WHERE scope IN ('', ?1) ORDER BY scope, id"
            ))?;
            let mut reg = Registry::default();
            for row in st.query_map([project], row_server)? {
                let (scope, s) = row?;
                if scope.is_empty() {
                    reg.global.insert(s.id.clone(), s);
                } else {
                    reg.project.insert(s.id.clone(), s);
                }
            }
            Ok(reg)
        })
    }

    /// Every entry of every scope.
    pub fn mcp_servers_all(&self) -> Result<Vec<(Scope, McpServer)>> {
        self.read(|c| {
            let mut st = c.prepare(&format!("SELECT {SERVER_COLS} FROM mcp_registry ORDER BY scope, id"))?;
            let rows = st.query_map([], row_server)?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows.into_iter().map(|(k, s)| (scope_of(k), s)).collect())
        })
    }

    pub fn mcp_server(&self, scope: &Scope, id: &str) -> Result<Option<McpServer>> {
        let (scope, id) = (scope_key(scope), id.to_string());
        self.read(move |c| {
            Ok(c.query_row(
                &format!("SELECT {SERVER_COLS} FROM mcp_registry WHERE scope=?1 AND id=?2"),
                [scope, id],
                row_server,
            )
            .optional()?
            .map(|(_, s)| s))
        })
    }

    /// Inserts or replaces an entry. A changed definition forgets the last
    /// probe (its status goes back to `unknown`).
    pub fn save_mcp_server(&self, scope: &Scope, server: &McpServer, at_ms: u64) -> Result<()> {
        let mut def = server.clone();
        def.status = McpStatus::Unknown;
        def.detail = None;
        def.checked_at_ms = None;
        def.used_by.clear();
        let scope = scope_key(scope);
        self.write(move |tx| {
            let old: Option<String> = tx
                .query_row("SELECT data FROM mcp_registry WHERE scope=?1 AND id=?2", params![scope, def.id], |r| {
                    r.get(0)
                })
                .optional()?;
            let data = js(&def);
            if old.as_deref() == Some(data.as_str()) {
                return Ok(());
            }
            tx.execute(
                "INSERT INTO mcp_registry(scope, id, data, status, detail, checked_at_ms, updated_at_ms)
                 VALUES (?1, ?2, ?3, 'unknown', NULL, NULL, ?4)
                 ON CONFLICT(scope, id) DO UPDATE SET data=excluded.data, status='unknown', detail=NULL,
                   checked_at_ms=NULL, updated_at_ms=excluded.updated_at_ms",
                params![scope, def.id, data, at_ms as i64],
            )?;
            Ok(())
        })
    }

    /// True when there was such an entry.
    pub fn delete_mcp_server(&self, scope: &Scope, id: &str) -> Result<bool> {
        let (scope, id) = (scope_key(scope), id.to_string());
        self.write(move |tx| Ok(tx.execute("DELETE FROM mcp_registry WHERE scope=?1 AND id=?2", [scope, id])? > 0))
    }

    /// Records a probe's outcome on the entry.
    pub fn set_mcp_status(
        &self,
        scope: &Scope,
        id: &str,
        status: McpStatus,
        detail: Option<&str>,
        at_ms: u64,
    ) -> Result<()> {
        let (scope, id, detail) = (scope_key(scope), id.to_string(), detail.map(str::to_string));
        self.write(move |tx| {
            tx.execute(
                "UPDATE mcp_registry SET status=?3, detail=?4, checked_at_ms=?5 WHERE scope=?1 AND id=?2",
                params![scope, id, status.as_str(), detail, at_ms as i64],
            )?;
            Ok(())
        })
    }

    /// Every role of every scope (archived ones left out).
    pub fn roles_all(&self) -> Result<Vec<RoleDef>> {
        self.read(|c| {
            let mut st = c.prepare("SELECT data FROM roles WHERE archived=0 ORDER BY scope, id")?;
            let rows = st.query_map([], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows.iter().map(|d| serde_json::from_str(d)).collect::<serde_json::Result<_>>()?)
        })
    }

    /// Every team of every scope (archived ones left out).
    pub fn teams_all(&self) -> Result<Vec<Team>> {
        self.read(|c| {
            let mut st = c.prepare("SELECT data FROM teams WHERE archived=0 ORDER BY scope, id")?;
            let rows = st.query_map([], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows.iter().map(|d| serde_json::from_str(d)).collect::<serde_json::Result<_>>()?)
        })
    }

    // ------------------------------------------------------------ integrations

    pub fn integrations(&self) -> Result<Vec<IntegrationRow>> {
        self.read(|c| {
            let mut st =
                c.prepare("SELECT id, status, detail, account, checked_at_ms FROM integrations ORDER BY id")?;
            let rows = st.query_map([], |r| {
                Ok(IntegrationRow {
                    id: r.get(0)?,
                    status: McpStatus::parse(&r.get::<_, String>(1)?),
                    detail: r.get(2)?,
                    account: r.get(3)?,
                    checked_at_ms: opt_u64(r, 4)?,
                })
            })?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        })
    }

    pub fn set_integration(&self, row: &IntegrationRow) -> Result<()> {
        let r = row.clone();
        self.write(move |tx| {
            tx.execute(
                "INSERT INTO integrations(id, status, detail, account, checked_at_ms) VALUES (?1,?2,?3,?4,?5)
                 ON CONFLICT(id) DO UPDATE SET status=excluded.status, detail=excluded.detail,
                   account=excluded.account, checked_at_ms=excluded.checked_at_ms",
                params![r.id, r.status.as_str(), r.detail, r.account, r.checked_at_ms.map(|v| v as i64)],
            )?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orch_team::mcp::McpKind;

    #[test]
    fn registry_is_seeded_scoped_and_probed() {
        let d = tempfile::tempdir().unwrap();
        let s = Store::open(&d.path().join("o.db")).unwrap();
        // Migration 4 seeds the penpot connector.
        let reg = s.mcp_registry(None).unwrap();
        assert_eq!(reg.global["penpot"], McpServer::penpot_seed());

        let mut f = McpServer::new("figma", "Figma", McpKind::Http);
        f.url = Some("https://figma.example/mcp".into());
        s.save_mcp_server(&Scope::Global, &f, 1).unwrap();
        let mut over = f.clone();
        over.url = Some("https://other.example/mcp".into());
        let p1 = Scope::Project("p1".into());
        s.save_mcp_server(&p1, &over, 2).unwrap();
        assert_eq!(s.mcp_registry(Some("p1")).unwrap().get("figma").unwrap().url, over.url);
        assert_eq!(s.mcp_registry(Some("p2")).unwrap().get("figma").unwrap().url, f.url);
        assert_eq!(s.mcp_servers_all().unwrap().len(), 3);

        s.set_mcp_status(&Scope::Global, "figma", McpStatus::NeedsAuth, Some("401"), 5).unwrap();
        let g = s.mcp_server(&Scope::Global, "figma").unwrap().unwrap();
        assert_eq!((g.status, g.detail.as_deref(), g.checked_at_ms), (McpStatus::NeedsAuth, Some("401"), Some(5)));
        // Saving the same definition keeps the probe; a change resets it.
        s.save_mcp_server(&Scope::Global, &g, 6).unwrap();
        assert_eq!(s.mcp_server(&Scope::Global, "figma").unwrap().unwrap().status, McpStatus::NeedsAuth);
        let mut g2 = g.clone();
        g2.name = "Figma 2".into();
        s.save_mcp_server(&Scope::Global, &g2, 7).unwrap();
        assert_eq!(s.mcp_server(&Scope::Global, "figma").unwrap().unwrap().status, McpStatus::Unknown);

        assert!(s.delete_mcp_server(&p1, "figma").unwrap());
        assert!(!s.delete_mcp_server(&p1, "figma").unwrap());
    }

    #[test]
    fn integrations_hold_status_only() {
        let d = tempfile::tempdir().unwrap();
        let s = Store::open(&d.path().join("o.db")).unwrap();
        assert!(s.integrations().unwrap().is_empty());
        let mut r = IntegrationRow {
            id: "linear".into(),
            status: McpStatus::Connected,
            detail: Some("logged in as G".into()),
            account: Some("Acme".into()),
            checked_at_ms: Some(9),
        };
        s.set_integration(&r).unwrap();
        r.status = McpStatus::NeedsAuth;
        s.set_integration(&r).unwrap();
        assert_eq!(s.integrations().unwrap(), [r]);
        let cols: Vec<String> = s
            .read(|c| {
                let mut st = c.prepare("SELECT name FROM pragma_table_info('integrations')")?;
                Ok(st.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?)
            })
            .unwrap();
        assert_eq!(cols, ["id", "status", "detail", "account", "checked_at_ms"], "no secret column");
    }
}
