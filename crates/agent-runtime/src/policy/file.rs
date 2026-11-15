//! `policy.toml` (api-contract §1): read, and written in place with
//! `toml_edit` (a rule or a sandbox allowance appended or removed, the
//! defaults set) keeping the user's comments and order.

use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use serde::Serialize;
use toml_edit::{ArrayOfTables, DocumentMut, Item, Table, value};

use super::sandbox::{SandboxEntry, SandboxKind};
use super::{Defaults, Effect, Rule};

/// What a new file starts with: both table kinds, one commented example each.
pub const HEADER: &str = "# Orchestrator permission policy (FR-1308). Settings > Permissions and Settings > Sandbox\n\
# edit this file for you; hand edits work too.\n\
#\n\
# Rules ([[rule]] tables) decide whether a tool call needs approval: match, effect\n\
# (allow | deny | overwatch | user), and optional id, role, agent, note.\n\
#   [[rule]]\n\
#   match = \"Bash(cargo test:*)\"\n\
#   effect = \"allow\"\n\
#   note = \"tests\"\n\
#\n\
# Sandbox allowances ([[sandbox]] tables) widen what Bash commands can reach: exactly one\n\
# of network, write, local_ports, unix_socket per table, and optional id, role, note.\n\
#   [[sandbox]]\n\
#   network = \"api.github.com\"        # a host, or *.example.com\n\
#   [[sandbox]]\n\
#   write = \"~/.cache/ms-playwright\"  # a folder Bash may write\n\
#   [[sandbox]]\n\
#   local_ports = true                # bind and listen on local ports\n\
#   [[sandbox]]\n\
#   unix_socket = \"/tmp/my-test.sock\" # connect to this Unix socket\n\
version = 1\n";

/// Where a file does not parse (1-based line).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ParseError {
    pub line: usize,
    pub message: String,
}

/// A parsed file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PolicyDoc {
    pub defaults: Option<Defaults>,
    pub rules: Vec<Rule>,
    /// `[[sandbox]]` tables, in file order (invalid ones flagged at load).
    pub sandbox: Vec<SandboxEntry>,
    /// `(id, message)`: duplicates, rules the hard lists neutralise,
    /// allowances that are never allowed.
    pub warnings: Vec<(String, String)>,
}

fn line_at(text: &str, offset: usize) -> usize {
    text[..offset.min(text.len())].matches('\n').count() + 1
}

fn str_of(t: &dyn toml_edit::TableLike, k: &str) -> Option<String> {
    t.get(k).and_then(Item::as_str).map(str::to_string)
}

/// Parses a policy file. An unknown key is ignored; a rule without `match`
/// or with an unknown effect is an error at its line.
pub fn parse(text: &str) -> Result<PolicyDoc, ParseError> {
    let doc = toml_edit::Document::parse(text.to_string()).map_err(|e| ParseError {
        line: e.span().map(|s| line_at(text, s.start)).unwrap_or(1),
        message: e.message().trim().to_string(),
    })?;
    let at = |span: Option<std::ops::Range<usize>>| span.map(|s| line_at(text, s.start)).unwrap_or(1);
    let bad = |line: usize, message: String| ParseError { line, message };
    if let Some(v) = doc.get("version") {
        if v.as_integer() != Some(1) {
            return Err(bad(at(v.span()), "version must be 1".into()));
        }
    }
    let mut out = PolicyDoc::default();
    if let Some(d) = doc.get("defaults") {
        let t = d.as_table_like().ok_or_else(|| bad(at(d.span()), "[defaults] must be a table".into()))?;
        let eff = |k: &str| -> Result<Option<Effect>, ParseError> {
            match t.get(k) {
                None => Ok(None),
                Some(i) => match i.as_str().and_then(Effect::parse_default) {
                    Some(e) => Ok(Some(e)),
                    None => Err(bad(at(i.span()), format!("defaults.{k} must be \"overwatch\" or \"user\""))),
                },
            }
        };
        let base = Defaults::default();
        out.defaults =
            Some(Defaults { team: eff("team")?.unwrap_or(base.team), solo: eff("solo")?.unwrap_or(base.solo) });
    }
    let empty = ArrayOfTables::new();
    let tables = match doc.get("rule") {
        None => &empty,
        Some(rules) => {
            rules.as_array_of_tables().ok_or_else(|| bad(at(rules.span()), "rule must be [[rule]] tables".into()))?
        }
    };
    let mut seen = std::collections::BTreeSet::new();
    for (i, t) in tables.iter().enumerate() {
        let line = at(t.span());
        let pattern = str_of(t, "match").ok_or_else(|| bad(line, "a rule needs match = \"Tool(…)\"".into()))?;
        let effect_raw = str_of(t, "effect").ok_or_else(|| bad(line, "a rule needs effect".into()))?;
        let effect = Effect::parse(&effect_raw)
            .ok_or_else(|| bad(line, format!("effect {effect_raw:?}: allow | deny | overwatch | user")))?;
        let id = str_of(t, "id").unwrap_or_else(|| generated_id('r', i, tables));
        if !seen.insert(id.clone()) {
            out.warnings.push((id.clone(), "duplicate id in this file: undo and lines may pick the other".into()));
        }
        out.rules.push(Rule {
            id,
            pattern,
            effect,
            role: str_of(t, "role"),
            agent: str_of(t, "agent"),
            note: str_of(t, "note"),
            from: str_of(t, "from"),
            added_ms: added_ms(t),
        });
    }
    let Some(sb) = doc.get("sandbox") else { return Ok(out) };
    let tables =
        sb.as_array_of_tables().ok_or_else(|| bad(at(sb.span()), "sandbox must be [[sandbox]] tables".into()))?;
    let mut seen = std::collections::BTreeSet::new();
    for (i, t) in tables.iter().enumerate() {
        let line = at(t.span());
        let present: Vec<SandboxKind> = SandboxKind::ALL.into_iter().filter(|k| t.contains_key(k.as_str())).collect();
        let [kind] = present[..] else {
            return Err(bad(
                line,
                "a [[sandbox]] needs exactly one of network, write, local_ports, unix_socket".into(),
            ));
        };
        let item = &t[kind.as_str()];
        let value = if kind.has_value() {
            Some(
                item.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| bad(line, format!("{} must be a string", kind.as_str())))?,
            )
        } else if item.as_bool() == Some(true) {
            None
        } else {
            return Err(bad(line, "local_ports must be true".into()));
        };
        let id = str_of(t, "id").unwrap_or_else(|| generated_id('s', i, tables));
        if !seen.insert(id.clone()) {
            out.warnings.push((id.clone(), "duplicate id in this file: Remove may pick the other".into()));
        }
        out.sandbox.push(SandboxEntry {
            id,
            kind,
            value,
            role: str_of(t, "role"),
            note: str_of(t, "note"),
            added_ms: added_ms(t),
            invalid: None,
        });
    }
    Ok(out)
}

fn added_ms(t: &Table) -> Option<u64> {
    t.get("added_ms").and_then(Item::as_integer).map(|n| n.max(0) as u64)
}

/// `<prefix><n>` for the n-th table (1-based) when it has no id; suffixed
/// when an explicit id already took it.
fn generated_id(prefix: char, i: usize, tables: &ArrayOfTables) -> String {
    let taken = |s: &str| tables.iter().any(|t| t.get("id").and_then(Item::as_str) == Some(s));
    let mut id = format!("{prefix}{}", i + 1);
    while taken(&id) {
        id.push('x');
    }
    id
}

fn read_doc(path: &Path) -> Result<DocumentMut> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => HEADER.to_string(),
        Err(e) => return Err(e).with_context(|| format!("read {}", path.display())),
    };
    if let Err(e) = parse(&text) {
        bail!("{} line {}: {} (fix it first)", path.display(), e.line, e.message);
    }
    text.parse::<DocumentMut>().map_err(|e| anyhow!("{}: {e}", path.display()))
}

/// Writes through a temp file and a rename: a watcher never reads half a file.
fn write_doc(path: &Path, doc: &DocumentMut) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    }
    crate::compile::write_atomic(path, doc.to_string().as_bytes()).with_context(|| format!("write {}", path.display()))
}

/// Creates the file with its header when missing. True if it created it.
pub fn ensure(path: &Path) -> Result<bool> {
    if path.exists() {
        return Ok(false);
    }
    write_doc(path, &HEADER.parse::<DocumentMut>()?)?;
    Ok(true)
}

/// Appends `rule` as a `[[rule]]` table (the file is created if missing).
pub fn append_rule(path: &Path, rule: &Rule) -> Result<()> {
    let mut doc = read_doc(path)?;
    let mut t = Table::new();
    t["id"] = value(&rule.id);
    t["match"] = value(&rule.pattern);
    t["effect"] = value(rule.effect.as_str());
    for (k, v) in [("role", &rule.role), ("agent", &rule.agent), ("note", &rule.note), ("from", &rule.from)] {
        if let Some(v) = v {
            t[k] = value(v.as_str());
        }
    }
    if let Some(ms) = rule.added_ms {
        t["added_ms"] = value(ms as i64);
    }
    push_table(&mut doc, "rule", t, path)?;
    write_doc(path, &doc)
}

/// `s` as a TOML string (Rust's `{:?}` escapes are not all TOML's).
fn toml_str(s: &str) -> String {
    toml_edit::Value::from(s).to_string().trim().to_string()
}

/// Appends `rule` to the file as a commented-out `[[rule]]` for the user to
/// edit and uncomment (Edit rule on a proposal). The file is created if missing.
pub fn append_commented_rule(path: &Path, rule: &Rule) -> Result<()> {
    ensure(path)?;
    let mut text = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str("\n# A rule proposed from your answers: remove the leading # of each line to turn it on.\n");
    text.push_str("# [[rule]]\n");
    text.push_str(&format!(
        "# id = {}\n# match = {}\n# effect = {}\n",
        toml_str(&rule.id),
        toml_str(&rule.pattern),
        toml_str(rule.effect.as_str())
    ));
    if let Some(r) = &rule.role {
        text.push_str(&format!("# role = {}\n", toml_str(r)));
    }
    if let Some(f) = &rule.from {
        text.push_str(&format!("# from = {}\n", toml_str(f)));
    }
    crate::compile::write_atomic(path, text.as_bytes()).with_context(|| format!("write {}", path.display()))
}

/// Removes the rule `id` (explicit or generated). False if absent.
pub fn remove_rule(path: &Path, id: &str) -> Result<bool> {
    remove_table(path, "rule", 'r', id)
}

/// Removes the `[[sandbox]]` `id` (explicit or generated). False if absent.
pub fn remove_sandbox(path: &Path, id: &str) -> Result<bool> {
    remove_table(path, "sandbox", 's', id)
}

fn remove_table(path: &Path, key: &str, prefix: char, id: &str) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let mut doc = read_doc(path)?;
    let Some(Item::ArrayOfTables(a)) = doc.get_mut(key) else { return Ok(false) };
    let pos = a.iter().position(|t| t.get("id").and_then(Item::as_str) == Some(id)).or_else(|| {
        (0..a.len()).find(|&i| a.get(i).is_some_and(|t| t.get("id").is_none()) && generated_id(prefix, i, a) == id)
    });
    let Some(i) = pos else { return Ok(false) };
    a.remove(i);
    if a.is_empty() {
        doc.remove(key);
    }
    write_doc(path, &doc)?;
    Ok(true)
}

/// Appends `e` as a `[[sandbox]]` table (the file is created if missing).
pub fn append_sandbox(path: &Path, e: &SandboxEntry) -> Result<()> {
    let mut doc = read_doc(path)?;
    let mut t = Table::new();
    t["id"] = value(&e.id);
    match &e.value {
        Some(v) => t[e.kind.as_str()] = value(v.as_str()),
        None => t[e.kind.as_str()] = value(true),
    }
    for (k, v) in [("role", &e.role), ("note", &e.note)] {
        if let Some(v) = v {
            t[k] = value(v.as_str());
        }
    }
    if let Some(ms) = e.added_ms {
        t["added_ms"] = value(ms as i64);
    }
    push_table(&mut doc, "sandbox", t, path)?;
    write_doc(path, &doc)
}

fn push_table(doc: &mut DocumentMut, key: &str, t: Table, path: &Path) -> Result<()> {
    match doc.get_mut(key) {
        Some(Item::ArrayOfTables(a)) => a.push(t),
        Some(_) => bail!("{}: `{key}` is not an array of tables", path.display()),
        None => {
            let mut a = ArrayOfTables::new();
            a.push(t);
            doc.insert(key, Item::ArrayOfTables(a));
        }
    }
    Ok(())
}

/// Sets `[defaults]` (created at the end when missing; its comments kept).
pub fn set_defaults(path: &Path, d: &Defaults) -> Result<()> {
    let mut doc = read_doc(path)?;
    if !doc.contains_key("defaults") {
        // Before the rules, so the file reads top-down.
        let mut t = Table::new();
        t.set_position(Some(0));
        doc.insert("defaults", Item::Table(t));
    }
    let t = doc["defaults"].as_table_mut().ok_or_else(|| anyhow!("[defaults] is not a table"))?;
    t["team"] = value(d.team.as_str());
    t["solo"] = value(d.solo.as_str());
    write_doc(path, &doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"# My policy: keep this comment.
version = 1

[defaults]          # team roles go to the Overwatch
team = "overwatch"
solo = "user"

# Installs are routine.
[[rule]]
id = "installs"
match = "Bash(cargo add:*)"
effect = "allow"
note = "routine installs"

[[rule]]
match = "Bash(rm:*)"
effect = "ask"   # read as user
role = "back"
"#;

    fn tmp() -> tempfile::TempDir {
        tempfile::Builder::new().prefix("pol").tempdir_in(orch_protocol::test_tmp()).unwrap()
    }

    #[test]
    fn parses_rules_defaults_and_ask_as_user() {
        let d = parse(SAMPLE).unwrap();
        assert_eq!(d.defaults, Some(Defaults { team: Effect::Overwatch, solo: Effect::User }));
        assert_eq!(d.rules.len(), 2);
        assert_eq!((d.rules[0].id.as_str(), d.rules[0].effect), ("installs", Effect::Allow));
        assert_eq!(d.rules[0].note.as_deref(), Some("routine installs"));
        assert_eq!((d.rules[1].id.as_str(), d.rules[1].effect), ("r2", Effect::User));
        assert_eq!(d.rules[1].role.as_deref(), Some("back"));
        assert_eq!(parse("").unwrap(), PolicyDoc::default());
    }

    #[test]
    fn errors_name_the_line() {
        let e = parse("version = 1\n\n[[rule]]\nmatch = \"Bash\"\neffect = \"maybe\"\n").unwrap_err();
        assert_eq!(e.line, 3);
        assert!(e.message.contains("maybe"), "{}", e.message);
        let e = parse("version = 1\nmatch = = 2\n").unwrap_err();
        assert_eq!(e.line, 2);
        let e = parse("[[rule]]\neffect = \"allow\"\n").unwrap_err();
        assert!(e.message.contains("match"));
        assert_eq!(parse("version = 2").unwrap_err().line, 1);
        assert!(parse("[defaults]\nteam = \"allow\"\n").unwrap_err().message.contains("defaults.team"));
    }

    #[test]
    fn duplicate_ids_warn() {
        let d = parse("[[rule]]\nid=\"a\"\nmatch=\"Bash\"\neffect=\"deny\"\n[[rule]]\nid=\"a\"\nmatch=\"Edit\"\neffect=\"deny\"\n")
            .unwrap();
        assert_eq!(d.warnings.len(), 1);
        // A generated id never takes an explicit one.
        let d =
            parse("[[rule]]\nmatch=\"Bash\"\neffect=\"deny\"\n[[rule]]\nid=\"r1\"\nmatch=\"Edit\"\neffect=\"deny\"\n")
                .unwrap();
        assert_eq!((d.rules[0].id.as_str(), d.rules[1].id.as_str()), ("r1x", "r1"));
    }

    #[test]
    fn writer_keeps_comments_and_order() {
        let d = tmp();
        let p = d.path().join("policy.toml");
        std::fs::write(&p, SAMPLE).unwrap();
        let rule = Rule {
            id: "r7".into(),
            pattern: "Bash(pnpm add zod)".into(),
            effect: Effect::Allow,
            from: Some("inbox-42".into()),
            added_ms: Some(1_793_600_000_000),
            ..Rule::default()
        };
        append_rule(&p, &rule).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.starts_with(SAMPLE), "the original text is untouched:\n{text}");
        assert!(text.contains("id = \"r7\""));
        let doc = parse(&text).unwrap();
        assert_eq!(doc.rules.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["installs", "r2", "r7"]);
        assert_eq!(doc.rules[2].from.as_deref(), Some("inbox-42"));
        assert_eq!(doc.rules[2].added_ms, Some(1_793_600_000_000));
        // Removing it gives the original back, byte for byte.
        assert!(remove_rule(&p, "r7").unwrap());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), SAMPLE);
        assert!(!remove_rule(&p, "r7").unwrap());
        // A generated id removes its rule; the comments of the others stay.
        assert!(remove_rule(&p, "r2").unwrap());
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("# My policy: keep this comment.") && text.contains("# Installs are routine."));
        assert_eq!(parse(&text).unwrap().rules.len(), 1);
    }

    const SANDBOX: &str = r#"version = 1

# Hosts for the release check.
[[sandbox]]
id = "s3"
network = "api.github.com"
role = "back"
note = "public GitHub API"

[[sandbox]]
write = "~/.cache/ms-playwright"   # browsers

[[sandbox]]
local_ports = true

[[sandbox]]
id = "s2"
unix_socket = "/tmp/my-test.sock"
"#;

    #[test]
    fn parses_sandbox_tables_with_positional_ids() {
        let d = parse(SANDBOX).unwrap();
        let got: Vec<_> = d.sandbox.iter().map(|e| (e.id.as_str(), e.kind, e.value.as_deref())).collect();
        assert_eq!(
            got,
            [
                ("s3", SandboxKind::Network, Some("api.github.com")),
                // Positional `s2` is taken by an explicit id: suffixed, like rules.
                ("s2x", SandboxKind::Write, Some("~/.cache/ms-playwright")),
                ("s3x", SandboxKind::LocalPorts, None),
                ("s2", SandboxKind::UnixSocket, Some("/tmp/my-test.sock")),
            ]
        );
        assert_eq!(d.sandbox[0].role.as_deref(), Some("back"));
        assert_eq!(d.sandbox[0].note.as_deref(), Some("public GitHub API"));
        // The header's examples are comments: it parses to nothing.
        let h = parse(HEADER).unwrap();
        assert!(h.rules.is_empty() && h.sandbox.is_empty());
    }

    #[test]
    fn sandbox_errors_name_the_line() {
        let two = "version = 1\n[[sandbox]]\nnetwork = \"a.com\"\nwrite = \"/tmp\"\n";
        let e = parse(two).unwrap_err();
        assert_eq!(e.line, 2);
        assert!(e.message.contains("exactly one"), "{}", e.message);
        assert!(parse("[[sandbox]]\nnote = \"x\"\n").unwrap_err().message.contains("exactly one"));
        assert!(parse("[[sandbox]]\nlocal_ports = false\n").unwrap_err().message.contains("true"));
        assert!(parse("[[sandbox]]\nnetwork = 3\n").unwrap_err().message.contains("string"));
        assert!(parse("sandbox = 1\n").unwrap_err().message.contains("[[sandbox]]"));
    }

    #[test]
    fn sandbox_writer_round_trip() {
        let d = tmp();
        let p = d.path().join("policy.toml");
        std::fs::write(&p, SANDBOX).unwrap();
        let e = SandboxEntry {
            id: "s9".into(),
            kind: SandboxKind::Network,
            value: Some("*.githubusercontent.com".into()),
            role: Some("front".into()),
            note: Some("raw \"files\"".into()),
            added_ms: Some(5),
            invalid: None,
        };
        append_sandbox(&p, &e).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.starts_with(SANDBOX), "{text}");
        assert_eq!(parse(&text).unwrap().sandbox.last(), Some(&e));
        let lp = SandboxEntry { id: "s10".into(), kind: SandboxKind::LocalPorts, value: None, ..e.clone() };
        append_sandbox(&p, &lp).unwrap();
        assert!(std::fs::read_to_string(&p).unwrap().contains("local_ports = true"));
        assert!(remove_sandbox(&p, "s10").unwrap() && remove_sandbox(&p, "s9").unwrap());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), SANDBOX);
        // A positional id removes its table; the others and the comments stay.
        assert!(remove_sandbox(&p, "s3x").unwrap());
        assert!(!remove_sandbox(&p, "s3x").unwrap());
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("# Hosts for the release check.") && text.contains("# browsers"));
        assert_eq!(parse(&text).unwrap().sandbox.len(), 3);
        // Rules and sandbox tables don't share ids.
        assert!(!remove_rule(&p, "s3").unwrap());
        // A new file: the header, then the table.
        let q = d.path().join("new/policy.toml");
        append_sandbox(&q, &e).unwrap();
        let text = std::fs::read_to_string(&q).unwrap();
        assert!(text.starts_with(HEADER));
        assert_eq!(parse(&text).unwrap().sandbox, [e]);
    }

    #[test]
    fn writer_creates_the_file_and_sets_defaults() {
        let d = tmp();
        let p = d.path().join("projects/x/policy.toml");
        assert!(!remove_rule(&p, "a").unwrap());
        set_defaults(&p, &Defaults { team: Effect::User, solo: Effect::User }).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.starts_with("# Orchestrator permission policy"), "{text}");
        assert_eq!(parse(&text).unwrap().defaults, Some(Defaults { team: Effect::User, solo: Effect::User }));
        // Existing defaults: changed in place, comment kept.
        let q = d.path().join("g.toml");
        std::fs::write(&q, SAMPLE).unwrap();
        set_defaults(&q, &Defaults { team: Effect::User, solo: Effect::Overwatch }).unwrap();
        let text = std::fs::read_to_string(&q).unwrap();
        assert!(text.contains("[defaults]          # team roles go to the Overwatch"), "{text}");
        assert!(text.contains("team = \"user\"") && text.contains("solo = \"overwatch\""));
        // A broken file is not written over.
        std::fs::write(&q, "[[rule]\n").unwrap();
        assert!(append_rule(&q, &Rule { id: "a".into(), pattern: "Bash".into(), ..Rule::default() }).is_err());
        assert_eq!(std::fs::read_to_string(&q).unwrap(), "[[rule]\n");
        assert!(ensure(&d.path().join("new/policy.toml")).unwrap());
        assert!(!ensure(&d.path().join("new/policy.toml")).unwrap());
    }
}
