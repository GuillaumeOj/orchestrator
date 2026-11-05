//! The Overwatch's tools (api-contract §2.2), reached only by the agent
//! `overwatch` (its role lists them; `on_mcp` routes here before the policy:
//! each tool checks the hard lists itself). B2 declares them and answers
//! `not_implemented`; B3 (T-225) implements the dispatch.

use serde_json::{Value, json};

use crate::mcp::McpAnswer;
use crate::runtime::Runtime;
use crate::store::AgentRecord;

/// An error answer `{code, message}` (api-contract §2.2 "Errors").
pub fn tool_error(code: &str, message: impl Into<String>) -> McpAnswer {
    let message = message.into();
    let v = json!({"code": code, "message": message});
    (true, v.to_string(), v)
}

impl Runtime {
    pub(crate) fn overwatch_tool(&self, _rec: &AgentRecord, tool: &str, _args: &Value) -> McpAnswer {
        tool_error("not_implemented", format!("{tool} is not available yet"))
    }
}
