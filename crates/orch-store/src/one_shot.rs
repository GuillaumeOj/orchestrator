//! One-shot allows (FR-705, ADR-009): the user approved a parked permission,
//! so the agent's retry of that exact call (agent + tool + canonical
//! `tool_input`) is allowed once, within its expiry.

use anyhow::Result;
use rusqlite::{OptionalExtension, params};
use serde_json::Value;

use crate::db::Store;

/// `tool_input` as one canonical string: serde_json's maps are sorted, so
/// key order never matters. The model's own `description` of the call is
/// left out: it does not change what runs, and a retry may word it anew.
fn canonical(input: &Value) -> String {
    let mut v = input.clone();
    if let Some(m) = v.as_object_mut() {
        m.remove("description");
    }
    v.to_string()
}

impl Store {
    /// Allows `agent`'s next `tool` call with exactly `input`, until `expires_at_ms`.
    pub fn add_one_shot(
        &self,
        agent_id: &str,
        tool: &str,
        input: &Value,
        item_id: Option<u64>,
        expires_at_ms: u64,
    ) -> Result<u64> {
        let (agent, tool, input) = (agent_id.to_string(), tool.to_string(), canonical(input));
        let now = crate::now_ms();
        self.write(move |tx| {
            tx.execute(
                "INSERT INTO one_shot_allows(agent_id, tool, tool_input, item_id, created_at_ms, expires_at_ms)
                 VALUES (?1,?2,?3,?4,?5,?6)",
                params![agent, tool, input, item_id.map(|i| i as i64), now as i64, expires_at_ms as i64],
            )?;
            Ok(tx.last_insert_rowid() as u64)
        })
    }

    /// Consumes the matching unexpired one-shot allow, if any; returns the
    /// Inbox item it came from (`Some(None)` when it had none).
    pub fn take_one_shot(&self, agent_id: &str, tool: &str, input: &Value) -> Result<Option<Option<u64>>> {
        let (agent, tool, input) = (agent_id.to_string(), tool.to_string(), canonical(input));
        let now = crate::now_ms() as i64;
        // Checked on every permission prompt: no write transaction unless one matches.
        let any = self.read(|c| {
            Ok(c.query_row(
                "SELECT 1 FROM one_shot_allows WHERE agent_id=?1 AND consumed_at_ms IS NULL AND expires_at_ms > ?2 LIMIT 1",
                params![agent, now],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
        })?;
        if !any {
            return Ok(None);
        }
        self.write(move |tx| {
            let hit: Option<(i64, Option<i64>)> = tx
                .query_row(
                    "SELECT id, item_id FROM one_shot_allows
                     WHERE agent_id=?1 AND tool=?2 AND tool_input=?3 AND consumed_at_ms IS NULL AND expires_at_ms > ?4
                     ORDER BY id LIMIT 1",
                    params![agent, tool, input, now],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            let Some((id, item)) = hit else { return Ok(None) };
            tx.execute("UPDATE one_shot_allows SET consumed_at_ms=?1 WHERE id=?2", params![now, id])?;
            Ok(Some(item.map(|i| i as u64)))
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn a_one_shot_matches_one_exact_call_once_before_it_expires() {
        let d = tempfile::tempdir().unwrap();
        let s = crate::Store::open(&d.path().join("s.db")).unwrap();
        let far = crate::now_ms() + 3_600_000;
        let input = json!({"command": "pnpm add zod", "description": "Add zod"});
        s.add_one_shot("a1", "Bash", &input, Some(7), far).unwrap();
        assert_eq!(s.take_one_shot("a2", "Bash", &input).unwrap(), None, "another agent");
        assert_eq!(s.take_one_shot("a1", "Write", &input).unwrap(), None, "another tool");
        assert_eq!(
            s.take_one_shot("a1", "Bash", &json!({"command": "pnpm add zod -D"})).unwrap(),
            None,
            "another input"
        );
        // Key order does not matter.
        let reordered: serde_json::Value =
            serde_json::from_str(r#"{"description":"Add zod","command":"pnpm add zod"}"#).unwrap();
        assert_eq!(s.take_one_shot("a1", "Bash", &reordered).unwrap(), Some(Some(7)));
        assert_eq!(s.take_one_shot("a1", "Bash", &input).unwrap(), None, "consumed");
        // The same call described in other words is the same call.
        s.add_one_shot("a1", "Bash", &input, Some(8), far).unwrap();
        let reworded = json!({"command": "pnpm add zod", "description": "Install zod"});
        assert_eq!(s.take_one_shot("a1", "Bash", &reworded).unwrap(), Some(Some(8)));
        s.add_one_shot("a1", "Bash", &input, None, crate::now_ms().saturating_sub(1)).unwrap();
        assert_eq!(s.take_one_shot("a1", "Bash", &input).unwrap(), None, "expired");
    }
}
