//! Scoped permission rules (FR-705, FR-708) evaluated by the core: for
//! PermissionRequest hooks (answered from the store or the Inbox) and for the
//! orchestrator's own MCP tools. Pure.
//!
//! Pattern syntax (a subset of Claude Code's): `Tool` matches every call of
//! the tool; `Tool(spec)` matches on the call's primary input (Bash command,
//! file path, URL…): `spec:*` or `spec*` is a prefix, `dir/**` a path prefix,
//! anything else is exact. A tool name ending in `*` is a prefix
//! (`mcp__penpot__*`). **Deny wins over ask, ask over allow** (all scopes).

use orch_store::{PermissionRule, RuleEffect, RuleScope};
use serde_json::Value;

/// Who is asking: rules apply when their scope matches any of these.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Subject {
    pub agent_id: String,
    pub role: String,
    pub project_id: Option<String>,
}

/// The input a rule's `(spec)` matches against.
pub fn primary_input(tool: &str, input: &Value) -> Option<String> {
    let key = match tool {
        "Bash" => "command",
        "Read" | "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => "file_path",
        "WebFetch" => "url",
        "WebSearch" => "query",
        "Glob" | "Grep" => "pattern",
        _ => {
            return match input {
                Value::Null => None,
                Value::Object(m) if m.is_empty() => None,
                other => Some(other.to_string()),
            };
        }
    };
    input.get(key).and_then(Value::as_str).map(str::to_string)
}

fn tool_matches(pat: &str, tool: &str) -> bool {
    match pat.strip_suffix('*') {
        Some(prefix) => tool.starts_with(prefix),
        None => pat == tool,
    }
}

/// Does `pattern` match this call?
pub fn matches(pattern: &str, tool: &str, input: &Value) -> bool {
    let pattern = pattern.trim();
    let (pt, spec) = match pattern.split_once('(') {
        Some((t, rest)) if rest.ends_with(')') => (t, Some(&rest[..rest.len() - 1])),
        _ => (pattern, None),
    };
    if !tool_matches(pt, tool) {
        return false;
    }
    let Some(spec) = spec else { return true };
    let Some(arg) = primary_input(tool, input) else { return false };
    if let Some(prefix) = spec.strip_suffix(":*") {
        // `git status:*` matches `git status` and `git status -s`, not `git statusx`.
        return arg == prefix || arg.starts_with(&format!("{prefix} "));
    }
    if let Some(dir) = spec.strip_suffix("/**") {
        let dir = dir.trim_start_matches("//");
        let arg = arg.trim_start_matches("//");
        let arg = arg.strip_prefix('/').unwrap_or(arg);
        let dir = dir.strip_prefix('/').unwrap_or(dir);
        return arg == dir || arg.starts_with(&format!("{dir}/"));
    }
    if let Some(prefix) = spec.strip_suffix('*') {
        return arg.starts_with(prefix);
    }
    arg == spec
}

fn applies(r: &PermissionRule, s: &Subject) -> bool {
    match r.scope {
        RuleScope::Agent => r.scope_id == s.agent_id,
        RuleScope::Role => r.scope_id == s.role,
        RuleScope::Project => s.project_id.as_deref() == Some(r.scope_id.as_str()),
    }
}

/// The deciding rule for a call, if any: deny, then ask, then allow.
pub fn evaluate<'a>(rules: &'a [PermissionRule], s: &Subject, tool: &str, input: &Value) -> Option<&'a PermissionRule> {
    let hits: Vec<&PermissionRule> =
        rules.iter().filter(|r| r.revoked_at_ms.is_none() && applies(r, s) && matches(&r.pattern, tool, input)).collect();
    [RuleEffect::Deny, RuleEffect::Ask, RuleEffect::Allow].iter().find_map(|e| hits.iter().find(|r| r.effect == *e).copied())
}

/// The rule "always allow this" creates: exact Bash command, the tool for the
/// others (a file tool's scope is the project already).
pub fn always_pattern(tool: &str, input: &Value) -> String {
    match (tool, primary_input(tool, input)) {
        ("Bash", Some(cmd)) if !cmd.contains(['(', ')']) => format!("Bash({cmd})"),
        _ => tool.to_string(),
    }
}

/// One line for the Inbox: `Bash: cargo test -p x`.
pub fn summary(tool: &str, input: &Value) -> String {
    let arg = primary_input(tool, input).unwrap_or_default();
    let mut s = if arg.is_empty() { tool.to_string() } else { format!("{tool}: {arg}") };
    if s.chars().count() > 200 {
        s = s.chars().take(199).collect::<String>() + "…";
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rule(id: u64, scope: RuleScope, sid: &str, pattern: &str, effect: RuleEffect) -> PermissionRule {
        PermissionRule {
            id,
            scope,
            scope_id: sid.into(),
            pattern: pattern.into(),
            effect,
            source_inbox_item_id: None,
            created_at_ms: 0,
            revoked_at_ms: None,
        }
    }

    #[test]
    fn pattern_matching() {
        let bash = |c: &str| json!({"command": c});
        assert!(matches("Bash", "Bash", &bash("rm -rf /")));
        assert!(matches("Bash(git status:*)", "Bash", &bash("git status -s")));
        assert!(matches("Bash(git status:*)", "Bash", &bash("git status")));
        assert!(!matches("Bash(git status:*)", "Bash", &bash("git statusx")));
        assert!(matches("Bash(npm *)", "Bash", &bash("npm test")));
        assert!(matches("Bash(cargo test)", "Bash", &bash("cargo test")));
        assert!(!matches("Bash(cargo test)", "Bash", &bash("cargo test -p x")));
        assert!(!matches("Bash", "Edit", &bash("x")));
        assert!(matches("Edit(//p/demo/src/**)", "Edit", &json!({"file_path": "/p/demo/src/a.rs"})));
        assert!(!matches("Edit(//p/demo/src/**)", "Edit", &json!({"file_path": "/p/demo/srcx/a.rs"})));
        assert!(matches("mcp__penpot__*", "mcp__penpot__export", &json!({})));
        assert!(matches("mcp__orch__commit", "mcp__orch__commit", &json!({"message": "x"})));
        assert!(!matches("WebFetch(https://a.example/*)", "WebFetch", &json!({})));
    }

    #[test]
    fn deny_beats_ask_beats_allow_across_scopes() {
        let s = Subject { agent_id: "a1".into(), role: "solo".into(), project_id: Some("demo".into()) };
        let input = json!({"command": "cargo test"});
        let mut rules = vec![
            rule(1, RuleScope::Project, "demo", "Bash", RuleEffect::Allow),
            rule(2, RuleScope::Project, "other", "Bash", RuleEffect::Deny),
        ];
        assert_eq!(evaluate(&rules, &s, "Bash", &input).unwrap().id, 1);
        rules.push(rule(3, RuleScope::Role, "solo", "Bash(cargo:*)", RuleEffect::Ask));
        assert_eq!(evaluate(&rules, &s, "Bash", &input).unwrap().id, 3);
        rules.push(rule(4, RuleScope::Agent, "a1", "Bash(cargo test)", RuleEffect::Deny));
        assert_eq!(evaluate(&rules, &s, "Bash", &input).unwrap().id, 4);
        rules[3].revoked_at_ms = Some(1);
        assert_eq!(evaluate(&rules, &s, "Bash", &input).unwrap().id, 3);
        assert!(evaluate(&rules, &s, "Edit", &json!({})).is_none());
    }

    #[test]
    fn always_patterns_and_summaries() {
        assert_eq!(always_pattern("Bash", &json!({"command": "cargo test"})), "Bash(cargo test)");
        assert_eq!(always_pattern("Edit", &json!({"file_path": "/x"})), "Edit");
        assert_eq!(summary("Bash", &json!({"command": "ls"})), "Bash: ls");
        assert_eq!(summary("mcp__x__y", &json!({})), "mcp__x__y");
    }
}
