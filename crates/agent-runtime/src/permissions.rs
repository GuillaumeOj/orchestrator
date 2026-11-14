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

/// The input key a rule's `(spec)` matches for a built-in tool.
pub fn primary_key(tool: &str) -> Option<&'static str> {
    Some(match tool {
        "Bash" => "command",
        "Read" | "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => "file_path",
        "WebFetch" => "url",
        "WebSearch" => "query",
        "Glob" | "Grep" => "pattern",
        _ => return None,
    })
}

/// The input a rule's `(spec)` matches against (the whole input for other tools).
pub fn primary_input(tool: &str, input: &Value) -> Option<String> {
    match primary_key(tool) {
        Some(key) => input.get(key).and_then(Value::as_str).map(str::to_string),
        None => match input {
            Value::Null => None,
            Value::Object(m) if m.is_empty() => None,
            other => Some(other.to_string()),
        },
    }
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
    // `WebFetch(domain:docs.rs)` (Claude Code's form): the URL's host; `*.x` its subdomains.
    if let Some(domain) = spec.strip_prefix("domain:").filter(|_| tool == "WebFetch") {
        return crate::policy::hard::url_host(&arg)
            .is_some_and(|h| crate::policy::hard::host_allowed(&[domain.to_string()], &h));
    }
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

/// The tools of the Settings rule form (`rule_preview`, `policy_test`).
pub const FORM_TOOLS: &[&str] = &["bash", "read", "edit", "webfetch", "mcp", "any"];

/// The host of `value` (`docs.rs`, `https://docs.rs/x`, `*.docs.rs`), lowercased.
fn form_host(value: &str) -> Result<String, String> {
    let v = value.trim();
    if let Some(rest) = v.strip_prefix("*.") {
        return form_host(rest).map(|h| format!("*.{h}"));
    }
    crate::policy::hard::url_host(v)
        .filter(|h| !h.is_empty() && h != "*")
        .ok_or_else(|| "Enter a domain, like docs.rs.".to_string())
}

/// The MCP tool name of `value` (`server__tool`, `server`, empty), without the `mcp__` prefix.
fn form_mcp(value: &str) -> Result<String, String> {
    let v = value.trim().trim_start_matches("mcp__").trim_end_matches('*').trim_end_matches("__");
    if !v.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-')) {
        return Err("Enter an MCP server, or server__tool, with letters, digits, _ and -.".into());
    }
    Ok(v.to_string())
}

/// `rule_preview`: the pattern for the Settings form's `tool` and `value`
/// (api-contract §3 Rules). The UI never builds patterns itself.
pub fn form_pattern(tool: &str, value: &str, home: &std::path::Path) -> Result<String, String> {
    let v = value.trim();
    Ok(match tool {
        "any" => "*".into(),
        "bash" => {
            let cmd = v.trim_end_matches('*').trim_end_matches(':').trim();
            if cmd.is_empty() {
                return Err("Enter the start of a command, like cargo test.".into());
            }
            if cmd.contains(['(', ')']) || cmd.contains('\n') {
                return Err("A command with ( ) or a line break can't be written as a rule.".into());
            }
            format!("Bash({cmd}:*)")
        }
        "webfetch" => format!("WebFetch(domain:{})", form_host(v)?),
        "read" | "edit" => {
            if v.is_empty() {
                return Err("Enter a folder or a file, like ~/notes.".into());
            }
            let p = crate::project_files::expand(v, home)?;
            let p = p.to_string_lossy().trim_end_matches('/').to_string();
            if p.is_empty() {
                return Err("/ is every file: pick a folder.".into());
            }
            if p.contains(['(', ')', '*', '?', '[', ']', '{', '}']) {
                return Err("A path with ( ) * ? [ ] { } can't be written as a rule.".into());
            }
            let tool = if tool == "read" { "Read" } else { "Edit" };
            format!("{tool}({}/**)", crate::compile::rule_path(std::path::Path::new(&p)))
        }
        "mcp" => match form_mcp(v)? {
            n if n.is_empty() => "mcp__*".into(),
            n if n.contains("__") => format!("mcp__{n}"),
            n => format!("mcp__{n}__*"),
        },
        other => return Err(format!("tool {other:?}: {}", FORM_TOOLS.join(" | "))),
    })
}

/// `policy_test`'s form: the tool name and input a call of `tool` with
/// `value` has (a full Bash command, a path, a domain, an MCP tool).
pub fn form_call(tool: &str, value: &str, home: &std::path::Path) -> Result<(String, Value), String> {
    let v = value.trim();
    Ok(match tool {
        "bash" => ("Bash".into(), serde_json::json!({"command": v})),
        "read" | "edit" => {
            let p = crate::project_files::expand(v, home)?;
            let name = if tool == "read" { "Read" } else { "Edit" };
            (name.into(), serde_json::json!({"file_path": p.display().to_string()}))
        }
        "webfetch" => {
            let h = form_host(v)?;
            ("WebFetch".into(), serde_json::json!({"url": format!("https://{}/", h.trim_start_matches("*."))}))
        }
        "mcp" => {
            let n = form_mcp(v)?;
            if !n.contains("__") {
                return Err("Enter the MCP tool as server__tool.".into());
            }
            (format!("mcp__{n}"), serde_json::json!({}))
        }
        other => return Err(format!("tool {other:?}: bash | read | edit | webfetch | mcp")),
    })
}

/// A rule's pattern in plain language (`policy_rules`' `summary`):
/// "Bash commands starting with `cargo test`".
pub fn rule_summary(pattern: &str) -> String {
    let p = pattern.trim();
    let (tool, spec) = match p.split_once('(') {
        Some((t, rest)) if rest.ends_with(')') => (t, Some(&rest[..rest.len() - 1])),
        _ => (p, None),
    };
    // `//abs` is absolute; `~/…`, `/…` (the settings file's folder) and
    // relative paths are shown as written.
    let path = |s: &str| s.strip_prefix('/').filter(|abs| abs.starts_with('/')).unwrap_or(s).to_string();
    let file_verb = |t: &str| match t {
        "Read" => Some("Reading"),
        "Edit" | "MultiEdit" => Some("Editing"),
        "Write" => Some("Writing"),
        _ => None,
    };
    match (tool, spec) {
        ("*", None) => "Any tool call".into(),
        ("Bash", None) => "Any Bash command".into(),
        ("Bash", Some(s)) => match s.strip_suffix(":*").or_else(|| s.strip_suffix('*')) {
            Some(prefix) => format!("Bash commands starting with `{}`", prefix.trim_end()),
            None => format!("The Bash command `{s}`"),
        },
        ("WebFetch", None) => "Fetching any web page".into(),
        ("WebFetch", Some(s)) => match s.strip_prefix("domain:") {
            Some(d) => format!("Fetching pages from `{d}`"),
            None => format!("Fetching `{s}`"),
        },
        (t, None) if file_verb(t).is_some() => format!("{} any file", file_verb(t).unwrap_or_default()),
        (t, Some(s)) if file_verb(t).is_some() => {
            let v = file_verb(t).unwrap_or_default();
            match s.strip_suffix("/**") {
                Some(dir) => format!("{v} files in `{}`", path(dir)),
                None => format!("{v} `{}`", path(s)),
            }
        }
        ("mcp__*", None) => "Any MCP tool".into(),
        (t, None) if t.starts_with("mcp__") => {
            let rest = &t["mcp__".len()..];
            match rest.strip_suffix("__*") {
                Some(server) => format!("Any tool of the `{server}` MCP server"),
                None => match rest.split_once("__") {
                    Some((server, name)) => format!("The `{name}` tool of the `{server}` MCP server"),
                    None => format!("MCP tools starting with `{}`", rest.trim_end_matches('*')),
                },
            }
        }
        (t, None) if t.ends_with('*') => format!("Tools starting with `{}`", t.trim_end_matches('*')),
        (t, None) => format!("Any `{t}` call"),
        (t, Some(s)) => format!("`{t}` calls matching `{s}`"),
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
    fn form_patterns_and_summaries() {
        let home = std::path::Path::new("/Users/u");
        let pat = |t: &str, v: &str| form_pattern(t, v, home);
        assert_eq!(pat("bash", " cargo test ").unwrap(), "Bash(cargo test:*)");
        assert_eq!(pat("bash", "cargo test:*").unwrap(), "Bash(cargo test:*)");
        assert_eq!(pat("webfetch", "docs.rs").unwrap(), "WebFetch(domain:docs.rs)");
        assert_eq!(pat("webfetch", "https://Docs.rs/x").unwrap(), "WebFetch(domain:docs.rs)");
        assert_eq!(pat("read", "~/notes/").unwrap(), "Read(//Users/u/notes/**)");
        assert_eq!(pat("edit", "/tmp/x").unwrap(), "Edit(//tmp/x/**)");
        assert_eq!(pat("mcp", "linear__save_issue").unwrap(), "mcp__linear__save_issue");
        assert_eq!(pat("mcp", "linear").unwrap(), "mcp__linear__*");
        assert_eq!(pat("mcp", "").unwrap(), "mcp__*");
        assert_eq!(pat("any", "ignored").unwrap(), "*");
        for (t, v) in [
            ("bash", ""),
            ("bash", "f(x)"),
            ("read", "rel"),
            ("read", "/"),
            ("webfetch", ""),
            ("mcp", "a b"),
            ("x", ""),
        ] {
            assert!(pat(t, v).is_err(), "{t} {v}");
        }
        // Each generated pattern matches the call it describes, and reads in words.
        let cases = [
            (
                "bash",
                "cargo test",
                "Bash",
                json!({"command": "cargo test -p x"}),
                "Bash commands starting with `cargo test`",
            ),
            (
                "webfetch",
                "docs.rs",
                "WebFetch",
                json!({"url": "https://docs.rs/serde"}),
                "Fetching pages from `docs.rs`",
            ),
            (
                "read",
                "~/notes",
                "Read",
                json!({"file_path": "/Users/u/notes/a.md"}),
                "Reading files in `/Users/u/notes`",
            ),
            ("mcp", "linear", "mcp__linear__save_issue", json!({}), "Any tool of the `linear` MCP server"),
            ("any", "", "Edit", json!({"file_path": "/x"}), "Any tool call"),
        ];
        for (t, v, tool, input, words) in cases {
            let p = pat(t, v).unwrap();
            assert!(matches(&p, tool, &input), "{p}");
            assert_eq!(rule_summary(&p), words);
        }
        assert!(!matches("WebFetch(domain:docs.rs)", "WebFetch", &json!({"url": "https://evil.com/docs.rs"})));
        assert!(matches("WebFetch(domain:*.docs.rs)", "WebFetch", &json!({"url": "https://a.docs.rs/"})));
        assert_eq!(rule_summary("Bash(cargo test)"), "The Bash command `cargo test`");
        assert_eq!(rule_summary("Bash"), "Any Bash command");
        assert_eq!(rule_summary("Edit(//a/b)"), "Editing `/a/b`");
        assert_eq!(rule_summary("Read(~/notes/**)"), "Reading files in `~/notes`");
        assert_eq!(rule_summary("Edit(src/**)"), "Editing files in `src`");
        assert_eq!(rule_summary("mcp__orch__commit"), "The `commit` tool of the `orch` MCP server");
        assert_eq!(rule_summary("mcp__*"), "Any MCP tool");
        assert_eq!(rule_summary("Glob"), "Any `Glob` call");
        // policy_test's calls.
        assert_eq!(form_call("bash", "git push", home).unwrap(), ("Bash".into(), json!({"command": "git push"})));
        assert_eq!(form_call("webfetch", "docs.rs", home).unwrap().1, json!({"url": "https://docs.rs/"}));
        assert_eq!(form_call("read", "~/a", home).unwrap().1, json!({"file_path": "/Users/u/a"}));
        assert_eq!(form_call("mcp", "orch__commit", home).unwrap().0, "mcp__orch__commit");
        assert!(form_call("mcp", "orch", home).is_err() && form_call("any", "", home).is_err());
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
