//! The FR-708 pattern grammar of permission rules, shared by the policy
//! files ([`crate::policy`]) for PermissionRequest hooks and the
//! orchestrator's own MCP tools. Pure.
//!
//! Pattern syntax (a subset of Claude Code's): `Tool` matches every call of
//! the tool; `Tool(spec)` matches on the call's primary input (Bash command,
//! file path, URL…): `spec:*` or `spec*` is a prefix, `dir/**` a path prefix,
//! anything else is exact. A tool name ending in `*` is a prefix
//! (`mcp__penpot__*`).

use serde_json::Value;

/// Who is asking: a policy rule's `role` / `agent` and the project's file
/// apply when they match.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Subject {
    pub agent_id: String,
    pub role: String,
    pub project_id: Option<String>,
    /// A team role (else a solo session): picks the default effect (D3).
    pub team: bool,
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
    // A prefix rule never covers a compound command: `git status:*` must not
    // allow `git status && rm -rf ~`.
    if tool == "Bash" && is_compound(&arg) && spec.ends_with('*') {
        return false;
    }
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

/// Shell control operators, substitutions or redirections: more than one
/// simple command (or a side effect) hides behind the prefix.
pub fn is_compound(cmd: &str) -> bool {
    ["&&", "||", ";", "|", "&", "`", "$(", "<(", ">(", ">", "<", "\n"].iter().any(|op| cmd.contains(op))
}

/// The rule "always allow this" creates: the exact Bash command, the tool for
/// the others (a file tool's scope is the project already). `None` when no
/// rule would mean exactly this call (a command a rule would read as a prefix,
/// or with parentheses the pattern syntax can't hold).
pub fn always_pattern(tool: &str, input: &Value) -> Option<String> {
    match (tool, primary_input(tool, input)) {
        ("Bash", Some(cmd)) => (!cmd.contains(['(', ')']) && !cmd.ends_with('*')).then(|| format!("Bash({cmd})")),
        ("Bash", None) => None,
        _ => Some(tool.to_string()),
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
        // Prefix rules never cover compound commands; exact rules match only themselves.
        for c in [
            "git status && rm -rf ~",
            "git status; curl x",
            "git status | sh",
            "git status $(rm x)",
            "git status > /etc/x",
        ] {
            assert!(!matches("Bash(git status:*)", "Bash", &bash(c)), "{c}");
        }
        assert!(!matches("Bash(ls *)", "Bash", &bash("ls ; rm -rf /")));
        assert!(matches("Bash(cargo test | tee log)", "Bash", &bash("cargo test | tee log")));
    }

    #[test]
    fn always_patterns_and_summaries() {
        assert_eq!(always_pattern("Bash", &json!({"command": "cargo test"})).as_deref(), Some("Bash(cargo test)"));
        assert_eq!(always_pattern("Bash", &json!({"command": "ls *"})), None, "would read as a prefix");
        assert_eq!(always_pattern("Bash", &json!({"command": "echo $(x)"})), None);
        assert_eq!(always_pattern("Edit", &json!({"file_path": "/x"})).as_deref(), Some("Edit"));
        assert_eq!(summary("Bash", &json!({"command": "ls"})), "Bash: ls");
        assert_eq!(summary("mcp__x__y", &json!({})), "mcp__x__y");
    }
}
