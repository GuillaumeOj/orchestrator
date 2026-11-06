//! The permission policy (FR-1308, spec §3.2): the core's hard lists
//! ([`hard`]), then the rules of the global and project `policy.toml` files
//! ([`file`]), then the default effect. The store's permission rules
//! (FR-708) were migrated into the files once (migration 15) and are no
//! longer read.
//!
//! Evaluation order: hard deny → policy rules (global and project merged;
//! the most restrictive effect wins: deny > user > overwatch > allow) → a
//! hard user-only call is lifted to `user` → no rule: `[defaults]` (team
//! roles `overwatch`, solo sessions `user`). The pattern grammar and the
//! compound-Bash rule are FR-708's ([`crate::permissions::matches`]).

pub mod file;
pub mod hard;
mod rt;
mod watch;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::permissions::{self, Subject};
use file::{ParseError, PolicyDoc};
use hard::{HardKind, HardVerdict};

pub(crate) use rt::Gate;
pub use rt::POLICY_ALERT;

/// What a rule (or the default) does with a call. Ordered by restriction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Effect {
    #[default]
    Allow,
    /// The Overwatch decides within its window, else the user (B2; held as `user` until then).
    Overwatch,
    User,
    Deny,
}

impl Effect {
    /// `ask` (the store's FR-708 effect) reads as `user`.
    pub fn parse(s: &str) -> Option<Effect> {
        Some(match s.trim() {
            "allow" => Effect::Allow,
            "overwatch" => Effect::Overwatch,
            "user" | "ask" => Effect::User,
            "deny" => Effect::Deny,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Effect::Allow => "allow",
            Effect::Overwatch => "overwatch",
            Effect::User => "user",
            Effect::Deny => "deny",
        }
    }

    /// A default effect (D3): `overwatch` | `user` only.
    pub fn parse_default(s: &str) -> Option<Effect> {
        Self::parse(s).filter(|e| matches!(e, Effect::Overwatch | Effect::User))
    }
}

/// One `[[rule]]` of a policy file.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Rule {
    pub id: String,
    #[serde(rename = "match")]
    pub pattern: String,
    pub effect: Effect,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Written by the core: `inbox-42` (Always allow), `store` (migrated).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub added_ms: Option<u64>,
}

impl Rule {
    fn applies(&self, s: &Subject) -> bool {
        let role_ok = self.role.as_deref().is_none_or(|r| r == s.role || (r == "solo" && !s.team));
        role_ok && self.agent.as_deref().is_none_or(|a| a == s.agent_id)
    }
}

/// The effect when no rule matches (decision D3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Defaults {
    pub team: Effect,
    pub solo: Effect,
}

impl Default for Defaults {
    fn default() -> Self {
        Defaults { team: Effect::Overwatch, solo: Effect::User }
    }
}

/// Which file.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Scope {
    Global,
    Project(String),
}

impl Scope {
    /// `global` | `project:<id>`: the file's label in sources and results.
    pub fn label(&self) -> String {
        match self {
            Scope::Global => "global".into(),
            Scope::Project(p) => format!("project:{p}"),
        }
    }

    pub fn parse(label: &str) -> Option<Scope> {
        match label {
            "global" => Some(Scope::Global),
            l => l.strip_prefix("project:").filter(|p| !p.is_empty()).map(|p| Scope::Project(p.into())),
        }
    }

    pub fn project(&self) -> Option<&str> {
        match self {
            Scope::Global => None,
            Scope::Project(p) => Some(p),
        }
    }
}

/// What decided the effect (`asks_because`, the audit's actor, B2's routing
/// and B3's learned-rule key read it).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
    /// A rule of a file: its id and the file's label.
    Policy { id: String, file: String },
    /// No rule matched: `[defaults]`.
    Default,
    /// The core's hard lists.
    Hard { why: &'static str },
}

/// A call's effect and what decided it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Evaluation {
    pub effect: Effect,
    pub source: Source,
    /// The deciding rule (`Source::Policy`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule: Option<Rule>,
}

impl Evaluation {
    /// The core's hard deny of a call.
    pub fn hard(h: &HardVerdict) -> Evaluation {
        let effect = if h.kind == HardKind::Deny { Effect::Deny } else { Effect::User };
        Evaluation { effect, source: Source::Hard { why: h.why }, rule: None }
    }

    /// Who denied, in words: "the core (git push: …)", "the user's policy rule r2 (Bash(rm:*))".
    pub fn denial(&self) -> String {
        match (&self.source, &self.rule) {
            (Source::Hard { why }, _) => format!("the core ({why})"),
            (_, Some(r)) => format!("the user's policy rule {} ({})", r.id, r.pattern),
            _ => "the policy".into(),
        }
    }

    /// "Always allow" can make a rule that takes effect: not when a `user`
    /// rule or the hard lists sent the call here (an allow never wins over them).
    pub fn allows_always(&self) -> bool {
        matches!(self.source, Source::Default)
    }

    /// The FR-709 actor of an answer it gives itself: `rule:<id>` | `core`.
    pub fn actor(&self) -> String {
        match &self.source {
            Source::Policy { id, .. } => format!("rule:{id}"),
            Source::Hard { .. } | Source::Default => "core".into(),
        }
    }

    /// The policy rule's id, if a rule decided.
    pub fn rule_id(&self) -> Option<&str> {
        match &self.source {
            Source::Policy { id, .. } => Some(id),
            _ => None,
        }
    }

    /// Why a held call waits on the user (api-contract §5 `asks_because`).
    pub fn asks_because(&self) -> Value {
        match &self.source {
            Source::Hard { why } => json!({"kind": "core", "detail": why}),
            Source::Policy { id, .. } => json!({"kind": "policy", "detail": id}),
            Source::Default => json!({"kind": "default", "detail": self.effect.as_str()}),
        }
    }
}

/// One file as loaded: its last good rules, and the parse error if the
/// current text is broken.
#[derive(Clone, Debug)]
pub struct FileState {
    pub scope: Scope,
    pub path: PathBuf,
    /// `path` through its directory's links (FSEvents reports real paths).
    real: PathBuf,
    pub exists: bool,
    pub doc: PolicyDoc,
    pub error: Option<ParseError>,
}

impl FileState {
    fn new(scope: Scope, path: PathBuf) -> FileState {
        let real = real_file(&path);
        FileState { scope, path, real, exists: false, doc: PolicyDoc::default(), error: None }
    }

    /// Reads the file again. A parse error keeps the last good rules.
    /// Returns whether the error changed (for the alert).
    fn reload(&mut self) -> bool {
        let before = self.error.clone();
        // Its directory may have appeared since.
        self.real = real_file(&self.path);
        match std::fs::read_to_string(&self.path) {
            Ok(text) => {
                self.exists = true;
                match file::parse(&text) {
                    Ok(mut doc) => {
                        doc.warnings.extend(neutralised(&doc.rules));
                        self.doc = doc;
                        self.error = None;
                    }
                    Err(e) => self.error = Some(e),
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                (self.exists, self.doc, self.error) = (false, PolicyDoc::default(), None);
            }
            Err(e) => self.error = Some(ParseError { line: 0, message: format!("not readable: {e}") }),
        }
        before != self.error
    }

    /// `policy_files` row (api-contract §3.3, §5 F15).
    pub fn row(&self) -> Value {
        let mut v = json!({
            "scope": if self.scope == Scope::Global { "global" } else { "project" },
            "file": self.scope.label(),
            "path": self.path.display().to_string(),
            "exists": self.exists,
            "rules": self.doc.rules.len(),
            "warnings": self.doc.warnings.iter().map(|(id, m)| json!({"id": id, "message": m})).collect::<Vec<_>>(),
        });
        if let Some(p) = self.scope.project() {
            v["project"] = json!(p);
        }
        if let Some(e) = &self.error {
            v["error"] = json!(e);
        }
        v
    }
}

fn real_file(p: &Path) -> PathBuf {
    match (p.parent(), p.file_name()) {
        (Some(d), Some(n)) => crate::paths::real(d).join(n),
        _ => p.to_path_buf(),
    }
}

/// Allow / overwatch rules whose match covers a call the hard lists keep:
/// loaded, but no effect on those calls (api-contract §1; Settings warns).
pub(crate) fn neutralised(rules: &[Rule]) -> Vec<(String, String)> {
    let ctx = hard::HardCtx {
        home: PathBuf::from("/nonexistent-home"),
        state_dir: PathBuf::new(),
        worktree: PathBuf::from("/nonexistent-worktree"),
        cwd: PathBuf::from("/nonexistent-worktree"),
        protected: vec!["main".into()],
        branch: std::cell::OnceCell::from(Some("main".to_string())),
        ..Default::default()
    };
    rules
        .iter()
        .filter(|r| matches!(r.effect, Effect::Allow | Effect::Overwatch))
        .filter_map(|r| {
            let p = r.pattern.trim();
            let (tool, spec) = match p.split_once('(') {
                Some((t, rest)) if rest.ends_with(')') => (t, Some(&rest[..rest.len() - 1])),
                _ => (p, None),
            };
            let bare = |s: &str| s.trim_end_matches('*').trim_end_matches(':').to_string();
            let v = match tool {
                "Bash" => {
                    // A prefix that covers `git push` (`Bash(git:*)`, `Bash(*)`) is tested as it.
                    let cmd = match spec {
                        Some(s) if !s.ends_with('*') || !"git push".starts_with(bare(s).as_str()) => bare(s),
                        _ => "git push".into(),
                    };
                    hard::check(&ctx, "Bash", &json!({"command": cmd}))
                }
                "WebFetch" => {
                    let url = spec.map(bare).unwrap_or_else(|| "https://example.invalid/".into());
                    hard::check(&ctx, "WebFetch", &json!({"url": url}))
                }
                t if t.starts_with("mcp__") => {
                    // `mcp__linear__*` → one of its tools; `mcp__*` (no server yet) → any server's.
                    let base = t.trim_end_matches('*');
                    let name = if !t.ends_with('*') {
                        t.to_string()
                    } else if base.matches("__").count() >= 2 {
                        format!("{base}write")
                    } else {
                        "mcp__x__write".to_string()
                    };
                    hard::check(&ctx, &name, &json!({}))
                }
                _ => None,
            }?;
            Some((r.id.clone(), format!("no effect on calls the core keeps ({})", v.why)))
        })
        .collect()
}

/// The pure evaluation over the files that apply (global first).
/// `use_defaults`: false for the orchestrator's own tools (no rule: run).
pub fn evaluate(
    files: &[&FileState],
    defaults: Defaults,
    s: &Subject,
    tool: &str,
    input: &Value,
    hard: Option<&HardVerdict>,
    use_defaults: bool,
) -> Option<Evaluation> {
    if let Some(h) = hard.filter(|h| h.kind == HardKind::Deny) {
        return Some(Evaluation::hard(h));
    }
    let mut best: Option<(&Rule, &Scope)> = None;
    for f in files {
        for r in &f.doc.rules {
            if r.applies(s)
                && permissions::matches(&r.pattern, tool, input)
                && best.is_none_or(|(b, _)| r.effect > b.effect)
            {
                best = Some((r, &f.scope));
            }
        }
    }
    let found = best.map(|(r, sc)| Evaluation {
        effect: r.effect,
        source: Source::Policy { id: r.id.clone(), file: sc.label() },
        rule: Some(r.clone()),
    });
    if let Some(h) = hard.filter(|h| h.kind == HardKind::UserOnly) {
        if found.as_ref().is_none_or(|e| e.effect < Effect::User) {
            return Some(Evaluation::hard(h));
        }
    }
    // A file that never parsed has no rules to read: its deny and user rules
    // are unknown, so nothing is allowed on its account (fail closed).
    let unread = files.iter().any(|f| f.error.is_some() && f.doc.rules.is_empty());
    if unread && found.as_ref().is_none_or(|e| e.effect < Effect::User) {
        return Some(Evaluation { effect: Effect::User, source: Source::Default, rule: None });
    }
    found.or_else(|| {
        use_defaults.then_some(Evaluation {
            effect: if s.team { defaults.team } else { defaults.solo },
            source: Source::Default,
            rule: None,
        })
    })
}

/// The global file's `[defaults]`, else D3's.
fn defaults_of(files: &BTreeMap<Scope, FileState>) -> Defaults {
    files.get(&Scope::Global).and_then(|f| f.doc.defaults).unwrap_or_default()
}

/// The loaded files, by scope.
#[derive(Default)]
pub struct Policy {
    files: std::sync::Mutex<BTreeMap<Scope, FileState>>,
    watch: std::sync::Mutex<Option<watch::Watch>>,
    /// Held across a file's read-modify-write (and the next id it picks):
    /// two concurrent writes neither lose a rule nor share an id.
    writes: std::sync::Mutex<()>,
}

impl Policy {
    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<Scope, FileState>> {
        self.files.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// One file write at a time (see `writes`).
    pub(crate) fn write_lock(&self) -> std::sync::MutexGuard<'_, ()> {
        self.writes.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Every loaded file, global first.
    pub fn files(&self) -> Vec<FileState> {
        self.lock().values().cloned().collect()
    }

    pub fn file(&self, scope: &Scope) -> Option<FileState> {
        self.lock().get(scope).cloned()
    }

    /// The global file's `[defaults]`, else D3's.
    pub fn defaults(&self) -> Defaults {
        defaults_of(&self.lock())
    }

    /// Is `scope`'s file known (read at a sync)?
    pub fn knows(&self, scope: &Scope) -> bool {
        self.lock().contains_key(scope)
    }

    /// The directories of the known files (for the watcher).
    fn dirs(&self) -> Vec<PathBuf> {
        self.lock().values().filter_map(|f| f.path.parent().map(PathBuf::from)).collect()
    }

    /// Sets the files to know (path per scope): new or moved ones are read,
    /// gone ones dropped. Returns the scopes read and whether their error changed.
    fn set_paths(&self, want: BTreeMap<Scope, PathBuf>) -> Vec<(Scope, bool)> {
        let mut files = self.lock();
        files.retain(|sc, _| want.contains_key(sc));
        let mut read = vec![];
        for (sc, path) in want {
            if files.get(&sc).is_some_and(|f| f.path == path) {
                continue;
            }
            let mut f = FileState::new(sc.clone(), path);
            let changed = f.reload();
            files.insert(sc.clone(), f);
            read.push((sc, changed));
        }
        read
    }

    /// Reads one file again; whether its error changed.
    fn reload(&self, scope: &Scope) -> Option<bool> {
        self.lock().get_mut(scope).map(FileState::reload)
    }

    /// The scope whose file is `path`.
    fn scope_of(&self, path: &Path) -> Option<Scope> {
        let want = real_file(path);
        self.lock().values().find(|f| f.path == path || f.real == want).map(|f| f.scope.clone())
    }

    /// The evaluation for `s` (its project's file and the global one).
    pub fn evaluate(
        &self,
        s: &Subject,
        tool: &str,
        input: &Value,
        hard: Option<&HardVerdict>,
        use_defaults: bool,
    ) -> Option<Evaluation> {
        let files = self.lock();
        let mut apply: Vec<&FileState> = files.get(&Scope::Global).into_iter().collect();
        if let Some(p) = &s.project_id {
            apply.extend(files.get(&Scope::Project(p.clone())));
        }
        evaluate(&apply, defaults_of(&files), s, tool, input, hard, use_defaults)
    }

    /// A rule by id (in `scope` when given), with its file.
    pub fn find(&self, id: &str, scope: Option<&Scope>) -> Option<(Scope, Rule)> {
        let files = self.lock();
        files
            .values()
            .filter(|f| scope.is_none_or(|s| *s == f.scope))
            .find_map(|f| f.doc.rules.iter().find(|r| r.id == id).map(|r| (f.scope.clone(), r.clone())))
    }

    /// The next free `r<n>` id across every file.
    pub fn next_id(&self) -> String {
        let n = self
            .lock()
            .values()
            .flat_map(|f| f.doc.rules.iter())
            .filter_map(|r| r.id.strip_prefix('r').and_then(|n| n.parse::<u64>().ok()))
            .max()
            .unwrap_or(0);
        format!("r{}", n + 1)
    }
}

#[cfg(test)]
mod tests;
