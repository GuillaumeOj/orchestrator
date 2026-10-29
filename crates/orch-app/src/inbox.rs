//! The Inbox's glue in the app (FR-707, §9.6): decisions are built in the
//! core ([`agent_runtime::decision`]) and pushed in the `now` event; this
//! keeps their listed ids and the P4.0 `inbox_outcome` event.

use serde_json::Value;

/// The listed id of a store Inbox item (`inbox-3`, `gate-5`, `q-6`…).
pub fn ui_id(kind: &str, id: u64) -> String {
    agent_runtime::decision::decision_id(kind, id)
}

/// An `inbox_outcome` event for the UI (api-contract §3): the runtime's
/// outcome with the item's listed id. Deprecated in P4.1 (the `now` event's
/// `decisions` carry every transition); removed in P4.3.
pub fn outcome_event(v: &Value) -> Value {
    let kind = v["kind"].as_str().unwrap_or("");
    serde_json::json!({
        "item": ui_id(kind, v["item"].as_u64().unwrap_or(0)),
        "decided_by": v["decided_by"], "effect": v["effect"], "at_ms": v["at_ms"],
    })
}
