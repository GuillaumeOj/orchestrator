//! Sandbox allowances (`[[sandbox]]` tables, api-contract §1–2): what a
//! user adds to an agent's OS sandbox, the validation every path shares (an
//! add, a check, a file read, a spawn: spec §3.1 "Never allowed"), and the
//! allowances an agent gets.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::compile::CREDENTIAL_STORES;
use crate::project_files::{UNSANDBOXED, UNSANDBOXED_NAMES, UNSANDBOXED_SYSTEM, fold, guarded};

/// What an allowance widens.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxKind {
    /// A host or `*.suffix`: `sandbox.network.allowedDomains`.
    Network,
    /// An absolute folder: `sandbox.filesystem.allowWrite`.
    Write,
    /// On/off: `sandbox.network.allowLocalBinding`.
    LocalPorts,
    /// An absolute socket path: `sandbox.network.allowUnixSockets`.
    UnixSocket,
}

impl SandboxKind {
    pub const ALL: [SandboxKind; 4] =
        [SandboxKind::Network, SandboxKind::Write, SandboxKind::LocalPorts, SandboxKind::UnixSocket];

    /// The kinds the installed CLI supports (`sandbox_kinds`). Checked
    /// against Claude Code 2.1.325's settings schema: `allowLocalBinding`,
    /// `allowUnixSockets` are read from a `--settings` file (README).
    pub const SUPPORTED: [SandboxKind; 4] = Self::ALL;

    /// Also the table's key in `policy.toml`.
    pub fn as_str(self) -> &'static str {
        match self {
            SandboxKind::Network => "network",
            SandboxKind::Write => "write",
            SandboxKind::LocalPorts => "local_ports",
            SandboxKind::UnixSocket => "unix_socket",
        }
    }

    pub fn parse(s: &str) -> Option<SandboxKind> {
        Self::ALL.into_iter().find(|k| k.as_str() == s.trim())
    }

    /// Whether the kind takes a value (all but `local_ports`).
    pub fn has_value(self) -> bool {
        self != SandboxKind::LocalPorts
    }
}

/// One `[[sandbox]]` table.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SandboxEntry {
    pub id: String,
    pub kind: SandboxKind,
    /// As written (`~` kept); `None` for `local_ports`.
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub added_ms: Option<u64>,
    /// Why it is never allowed (set at load): skipped, listed in the file's warnings.
    #[serde(skip)]
    pub invalid: Option<String>,
}

impl SandboxEntry {
    /// For agents of `role` (absent: every role). The Overwatch gets none (the caller's).
    pub fn applies(&self, role: &str) -> bool {
        self.invalid.is_none() && self.role.as_deref().is_none_or(|r| r == role)
    }

    /// Same kind, value and role: a duplicate in its file.
    pub fn same(&self, kind: SandboxKind, value: Option<&str>, role: Option<&str>) -> bool {
        self.kind == kind && self.value.as_deref() == value && self.role.as_deref() == role
    }
}

/// A registered project, as the guard sees it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GuardProject {
    pub id: String,
    pub checkout: PathBuf,
    pub worktree_root: PathBuf,
}

/// What no allowance may touch (spec §3.1). Paths are canonical.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Guard {
    pub home: PathBuf,
    pub app_root: PathBuf,
    pub projects: Vec<GuardProject>,
    /// Credential stores and where the symlinked ones point.
    pub credentials: Vec<PathBuf>,
    /// Code run outside the sandbox (project_files' list).
    pub unsandboxed: Vec<PathBuf>,
    /// `$SSH_AUTH_SOCK`, canonical.
    pub ssh_agent: Option<PathBuf>,
}

impl Guard {
    pub fn new(home: &Path, app_root: &Path, projects: Vec<GuardProject>, ssh_agent: Option<PathBuf>) -> Guard {
        let home = real_lenient(home);
        Guard {
            credentials: guarded(&home, CREDENTIAL_STORES.iter().map(|(p, _)| *p)),
            unsandboxed: guarded(&home, UNSANDBOXED.iter().chain(UNSANDBOXED_SYSTEM).copied()),
            app_root: real_lenient(app_root),
            projects: projects
                .into_iter()
                .map(|p| GuardProject {
                    checkout: real_lenient(&p.checkout),
                    worktree_root: real_lenient(&p.worktree_root),
                    id: p.id,
                })
                .collect(),
            ssh_agent: ssh_agent.map(|p| real_lenient(&p)),
            home,
        }
    }
}

/// System folders no agent writes (the temp folders stay open).
const SYSTEM: &[&str] =
    &["/System", "/usr", "/bin", "/sbin", "/etc", "/private/etc", "/Library", "/var", "/private/var", "/cores", "/dev"];
const SYSTEM_OPEN: &[&str] = &["/var/folders", "/private/var/folders", "/tmp", "/private/tmp"];

/// Claude Code's sandbox paths are patterns: these would be read as one.
const PATTERN_CHARS: &[char] = &['*', '?', '[', ']', '{', '}', '\\', '"'];

/// `p` through its links; a missing tail is kept as typed after the
/// nearest existing ancestor's real path (a cache folder not created yet).
pub fn real_lenient(p: &Path) -> PathBuf {
    let mut tail = vec![];
    let mut cur = p.to_path_buf();
    loop {
        if let Ok(r) = std::fs::canonicalize(&cur) {
            return tail.into_iter().rev().fold(r, |acc: PathBuf, c| acc.join(c));
        }
        match (cur.parent(), cur.file_name()) {
            (Some(parent), Some(name)) => {
                tail.push(name.to_os_string());
                cur = parent.to_path_buf();
            }
            _ => return p.to_path_buf(),
        }
    }
}

/// `~`, `~/…` expanded; `..` refused; the result absolute.
fn expand(raw: &str, home: &Path) -> Result<PathBuf, String> {
    let p = if raw == "~" {
        home.to_path_buf()
    } else if let Some(rest) = raw.strip_prefix("~/") {
        home.join(rest)
    } else {
        PathBuf::from(raw)
    };
    if !p.is_absolute() {
        return Err(format!("{raw} is not an absolute path: start it with / or ~/."));
    }
    if p.components().any(|c| c == std::path::Component::ParentDir) {
        return Err(format!("{raw} contains ..: write the folder's full path."));
    }
    Ok(p)
}

/// A host (`api.github.com`) or a wildcard (`*.githubusercontent.com`), lowercased.
fn check_host(raw: &str) -> Result<String, String> {
    let h = raw.trim().trim_end_matches('.').to_lowercase();
    if h.is_empty() {
        return Err("Enter a host, like api.github.com.".into());
    }
    if h == "*" || h == "*." {
        return Err("All hosts (*) is never allowed: name the host the agent needs.".into());
    }
    if h.contains("://") {
        return Err(format!("Enter the host only, without {}://.", h.split("://").next().unwrap_or("")));
    }
    if h.contains(['/', ':', '@', '?', '#']) || h.chars().any(char::is_whitespace) {
        return Err("Enter a host only, without a path, a port or spaces.".into());
    }
    let bare = h.strip_prefix("*.").unwrap_or(&h);
    let label_ok = |l: &str| {
        !l.is_empty()
            && !l.starts_with('-')
            && !l.ends_with('-')
            && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    };
    if !bare.split('.').all(label_ok) {
        return Err(format!(
            "{h} is not a host name: use letters, digits, - and dots, with an optional *. at the start."
        ));
    }
    if h.starts_with("*.") && !bare.contains('.') {
        return Err(format!("*.{bare} covers a whole top-level domain: name a domain under it."));
    }
    Ok(h)
}

/// The shared path checks (write folders and sockets). The message is a
/// full sentence for the user (the UI shows it verbatim).
fn check_common(shown: &str, canon: &Path, g: &Guard) -> Result<(), String> {
    let c = fold(canon);
    let inside = |outer: &Path| c.starts_with(fold(outer));
    let contains = |inner: &Path| fold(inner).starts_with(&c);
    let overlaps = |list: &[PathBuf]| list.iter().any(|p| inside(p) || contains(p));
    let s = canon.to_string_lossy();
    if s.contains(PATTERN_CHARS) || s.chars().any(char::is_control) {
        return Err(format!("{shown}: names with * ? [ ] {{ }} \\ \" or control characters can't be allowed."));
    }
    if canon.parent().is_none() || contains(&g.home) {
        return Err(format!("{shown} is too broad: pick a folder inside it."));
    }
    if overlaps(&g.credentials) {
        return Err(format!("{shown} holds credentials: never allowed."));
    }
    if inside(&g.app_root) || contains(&g.app_root) {
        return Err(format!("{shown} is the orchestrator's own data: never allowed."));
    }
    if canon.components().any(|x| UNSANDBOXED_NAMES.iter().any(|n| x.as_os_str() == *n)) || overlaps(&g.unsandboxed) {
        return Err(format!("{shown} holds settings or code that runs outside the sandbox: never allowed."));
    }
    Ok(())
}

/// A write folder: [`check_common`], then the repositories, worktrees and
/// system folders. `project`: the file's project (its own worktrees).
fn check_write(shown: &str, canon: &Path, g: &Guard, project: Option<&str>) -> Result<(), String> {
    check_common(shown, canon, g)?;
    let c = fold(canon);
    let inside = |outer: &Path| c.starts_with(fold(outer));
    let contains = |inner: &Path| fold(inner).starts_with(&c);
    for p in &g.projects {
        if inside(&p.worktree_root) {
            return Err(if project == Some(p.id.as_str()) {
                format!(
                    "{shown} is already writable: it's the agent's worktree (another agent's worktree is never allowed)."
                )
            } else {
                format!("{shown} is inside another project's agent worktrees: never allowed.")
            });
        }
        if inside(&p.checkout) {
            return Err(format!(
                "{shown} is inside a project's main checkout: never allowed (agents work in their worktree)."
            ));
        }
        if contains(&p.checkout) || contains(&p.worktree_root) {
            return Err(format!("{shown} contains a project repository or agent worktrees: pick a folder inside it."));
        }
    }
    let open = SYSTEM_OPEN.iter().any(|o| inside(Path::new(o)));
    if !open && SYSTEM.iter().any(|sys| inside(Path::new(sys)) || contains(Path::new(sys))) {
        return Err(format!("{shown} is a system folder: never allowed."));
    }
    Ok(())
}

/// A socket: [`check_common`], then the services that run commands or sign
/// in outside the sandbox.
fn check_socket(shown: &str, canon: &Path, g: &Guard) -> Result<(), String> {
    check_common(shown, canon, g)?;
    let s = canon.to_string_lossy().to_lowercase();
    let name = canon.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    let home = g.home.to_string_lossy().to_lowercase();
    let docker_dirs = [".docker", ".colima", ".orbstack", ".rd", ".lima"];
    if name.contains("docker")
        || name.contains("podman")
        || docker_dirs.iter().any(|d| s.starts_with(&format!("{home}/{d}/")))
    {
        return Err(format!(
            "{shown} is a container engine's socket: it runs commands outside the sandbox, never allowed."
        ));
    }
    let ssh = g.ssh_agent.as_ref().is_some_and(|a| fold(a) == fold(canon));
    if ssh || name.contains("ssh") || name.contains("gpg-agent") || s.contains("/com.apple.launchd.") {
        return Err(format!("{shown} is a key agent's socket: it signs in with your keys, never allowed."));
    }
    Ok(())
}

/// Validates an allowance (spec §3.1 "Never allowed"). `Ok`: the value as
/// stored (hosts lowercased, paths trimmed with `~` kept; `None` for
/// `local_ports`); `Err`: the reason, a sentence for the user.
pub fn validate(
    kind: SandboxKind,
    raw: Option<&str>,
    g: &Guard,
    project: Option<&str>,
) -> Result<Option<String>, String> {
    let raw = raw.map(str::trim).filter(|s| !s.is_empty());
    match kind {
        SandboxKind::LocalPorts => Ok(None),
        SandboxKind::Network => check_host(raw.unwrap_or("")).map(Some),
        SandboxKind::Write | SandboxKind::UnixSocket => {
            let raw = raw.ok_or_else(|| {
                if kind == SandboxKind::Write {
                    "Enter a folder, like ~/.cache/ms-playwright."
                } else {
                    "Enter a socket path."
                }
                .to_string()
            })?;
            resolve(kind, raw, g, project)?;
            Ok(Some(raw.to_string()))
        }
    }
}

/// The canonical path a valid `write` / `unix_socket` value compiles to
/// (re-validated: links may have moved since it was added).
pub fn resolve(kind: SandboxKind, raw: &str, g: &Guard, project: Option<&str>) -> Result<PathBuf, String> {
    let canon = real_lenient(&expand(raw, &g.home)?);
    match kind {
        SandboxKind::Write => check_write(raw, &canon, g, project)?,
        SandboxKind::UnixSocket => check_socket(raw, &canon, g)?,
        _ => return Err(format!("{} has no path", kind.as_str())),
    }
    Ok(canon)
}

/// What the allowances add to an agent's sandbox (`compile` and the hard
/// list put them after the role's own hosts).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Allowances {
    pub network: Vec<String>,
    pub write: Vec<PathBuf>,
    pub local_binding: bool,
    pub unix_sockets: Vec<PathBuf>,
}

impl Allowances {
    /// From `entries` (global first, then the project's), for `role`;
    /// paths re-validated and canonicalised, invalid ones skipped.
    pub fn of<'a>(
        entries: impl IntoIterator<Item = &'a SandboxEntry>,
        role: &str,
        g: &Guard,
        project: Option<&str>,
    ) -> Allowances {
        let mut out = Allowances::default();
        for e in entries.into_iter().filter(|e| e.applies(role)) {
            let v = e.value.as_deref().unwrap_or("");
            match e.kind {
                SandboxKind::Network => push_new(&mut out.network, v.to_string()),
                SandboxKind::LocalPorts => out.local_binding = true,
                SandboxKind::Write | SandboxKind::UnixSocket => match resolve(e.kind, v, g, project) {
                    Ok(p) if e.kind == SandboxKind::Write => push_new(&mut out.write, p),
                    Ok(p) => push_new(&mut out.unix_sockets, p),
                    Err(why) => eprintln!("warn: sandbox entry {} skipped: {why}", e.id),
                },
            }
        }
        out
    }

    /// `base` (defaults and the role's hosts) followed by the allowed hosts, deduplicated.
    pub fn network_with(&self, base: &[String]) -> Vec<String> {
        let mut out = vec![];
        for h in base.iter().chain(&self.network) {
            push_new(&mut out, h.clone());
        }
        out
    }
}

fn push_new<T: PartialEq>(v: &mut Vec<T>, x: T) {
    if !v.contains(&x) {
        v.push(x);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guard(d: &Path) -> Guard {
        let home = d.join("home");
        for p in ["home/.ssh", "home/.cache", "app", "repo/.git", "wt/p1/back-1", "other/repo", "other-wt"] {
            std::fs::create_dir_all(d.join(p)).unwrap();
        }
        Guard::new(
            &home,
            &d.join("app"),
            vec![
                GuardProject { id: "p1".into(), checkout: d.join("repo"), worktree_root: d.join("wt") },
                GuardProject { id: "p2".into(), checkout: d.join("other/repo"), worktree_root: d.join("other-wt") },
            ],
            Some(d.join("agent.sock")),
        )
    }

    #[test]
    fn validation_table() {
        let d = tempfile::Builder::new().prefix("sbx").tempdir_in(orch_protocol::test_tmp()).unwrap();
        let g = guard(d.path());
        let dp = d.path().display().to_string();
        let ok = |k: SandboxKind, v: &str| validate(k, Some(v), &g, Some("p1"));
        use SandboxKind::*;
        // Accepted, normalised.
        assert_eq!(ok(Network, " API.GitHub.com. ").unwrap().as_deref(), Some("api.github.com"));
        assert_eq!(ok(Network, "*.githubusercontent.com").unwrap().as_deref(), Some("*.githubusercontent.com"));
        assert_eq!(ok(Network, "127.0.0.1").unwrap().as_deref(), Some("127.0.0.1"));
        assert_eq!(ok(Write, "~/.cache/ms-playwright").unwrap().as_deref(), Some("~/.cache/ms-playwright"));
        assert!(ok(Write, "/private/tmp/x").is_ok() && ok(Write, "/var/folders/ab/T").is_ok());
        assert!(ok(UnixSocket, "/tmp/my-test.sock").is_ok());
        assert_eq!(validate(LocalPorts, None, &g, None).unwrap(), None);
        // Refused, with the reason.
        let no = |k: SandboxKind, v: &str, why: &str| {
            let e = ok(k, v).unwrap_err();
            assert!(e.contains(why), "{v}: {e}");
        };
        no(Network, "", "Enter a host");
        no(Network, "*", "never allowed");
        no(Network, "https://github.com", "without https://");
        no(Network, "github.com/x", "without a path");
        no(Network, "github.com:443", "without a path, a port");
        no(Network, "*.com", "top-level domain");
        no(Network, "a_b.com", "not a host name");
        no(Write, "", "Enter a folder");
        no(Write, "relative/dir", "absolute");
        no(Write, "/", "too broad");
        no(Write, "~", "too broad");
        no(Write, &dp, "too broad");
        no(Write, "~/.ssh", "credentials");
        no(Write, "~/.ssh/keys", "credentials");
        no(Write, "~/.claude", "outside the sandbox");
        no(Write, "~/.zshrc", "outside the sandbox");
        no(Write, "~/x/.git/hooks", "outside the sandbox");
        no(Write, &format!("{dp}/app/state"), "orchestrator's own data");
        no(Write, &format!("{dp}/repo/src"), "main checkout");
        no(Write, &format!("{dp}/wt/p1/back-1"), "already writable: it's the agent's worktree");
        no(Write, &format!("{dp}/other-wt/a"), "another project's");
        no(Write, &format!("{dp}/other"), "contains a project repository");
        no(Write, "/usr/lib", "system folder");
        no(Write, "/etc", "system folder");
        no(Write, "/opt/homebrew/bin", "outside the sandbox");
        no(Write, "~/a/../.ssh", "contains ..");
        no(Write, "/tmp/a*b", "can't be allowed");
        no(UnixSocket, "/var/run/docker.sock", "container engine");
        no(UnixSocket, "~/.colima/default/x.sock", "container engine");
        no(UnixSocket, &format!("{dp}/agent.sock"), "key agent");
        no(UnixSocket, "/private/tmp/com.apple.launchd.abc/Listeners", "key agent");
        no(UnixSocket, &format!("{dp}/app/orchd.sock"), "orchestrator's own data");
        // In the global file, a worktree root is no agent's own.
        assert!(validate(Write, Some(&format!("{dp}/wt/p1")), &g, None).unwrap_err().contains("another project's"));
    }

    #[test]
    fn allowances_keep_order_dedupe_and_filter_roles() {
        let d = tempfile::Builder::new().prefix("sbx").tempdir_in(orch_protocol::test_tmp()).unwrap();
        let g = guard(d.path());
        let e = |id: &str, kind, v: Option<&str>, role: Option<&str>| SandboxEntry {
            id: id.into(),
            kind,
            value: v.map(str::to_string),
            role: role.map(str::to_string),
            note: None,
            added_ms: None,
            invalid: None,
        };
        let mut bad = e("s9", SandboxKind::Network, Some("evil.com"), None);
        bad.invalid = Some("x".into());
        let entries = [
            e("s1", SandboxKind::Network, Some("api.github.com"), None),
            e("s2", SandboxKind::Network, Some("crates.io"), None),
            e("s3", SandboxKind::Write, Some("~/.cache/pw"), Some("front")),
            e("s4", SandboxKind::LocalPorts, None, Some("back")),
            e("s5", SandboxKind::UnixSocket, Some("/tmp/t.sock"), None),
            e("s6", SandboxKind::Network, Some("api.github.com"), None),
            bad,
        ];
        let back = Allowances::of(&entries, "back", &g, Some("p1"));
        assert_eq!(back.network, ["api.github.com", "crates.io"]);
        assert!(back.local_binding && back.write.is_empty());
        assert_eq!(back.unix_sockets, [real_lenient(Path::new("/tmp/t.sock"))]);
        let front = Allowances::of(&entries, "front", &g, Some("p1"));
        assert!(!front.local_binding);
        assert_eq!(front.write, [real_lenient(&d.path().join("home/.cache/pw"))]);
        assert_eq!(
            back.network_with(&["crates.io".into(), "example.org".into()]),
            ["crates.io", "example.org", "api.github.com"]
        );
    }
}
