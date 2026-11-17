//! The gh login a GitHub tracker needs (fix-write-back-github spec §4.2–4.3, api-contract §2–3):
//! `tracker_gh_access` compares the scopes the tracker needs with what `gh auth status` lists;
//! `gh_grant_scope` runs gh's device flow (`gh auth refresh -h github.com -s <scope>`) for the
//! user, answers the one-time code at once and posts `gh.grant` when it ends, then retries the
//! failed write-backs of every GitHub-tracker project.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};

use anyhow::Result;
use orch_store::Tracker;
use serde_json::{Value, json};

use crate::net::{GhRefresh, RefreshChild};
use crate::runtime::Runtime;

/// The scopes `gh_grant_scope` may ask for (anything else could add scopes or flags to gh's command line).
const GRANTABLE: [&str; 2] = ["project", "read:project"];
/// How long gh may take to print the code before the app gives up on running it.
const CODE_WAIT: Duration = Duration::from_secs(10);
/// A device code lives about 15 minutes: then the grant is killed.
const GRANT_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const POLL: Duration = Duration::from_millis(200);
/// How long, once gh exited, its last lines may take to come through the reader threads.
const DRAIN: Duration = Duration::from_millis(500);
/// github.com's device-flow page.
const DEVICE_URL: &str = "https://github.com/login/device";

/// Reads what is left of an exited gh's output: until both streams end (or nothing comes for [`DRAIN`]).
fn drain_after_exit(lines: &std::sync::mpsc::Receiver<String>, mut f: impl FnMut(String)) {
    while let Ok(l) = lines.recv_timeout(DRAIN) {
        f(l);
    }
}

/// gh's output lines while it starts: the one-time code and the device URL once seen.
#[derive(Default)]
struct Scan {
    code: Option<String>,
    url: Option<String>,
    seen: Vec<String>,
}

impl Scan {
    fn push(&mut self, l: String) {
        if self.code.is_none() {
            self.code = device_code(&l);
        }
        if self.url.is_none() {
            self.url = device_url(&l);
        }
        self.seen.push(l);
    }
}

/// Told of every `gh.grant` outcome ([`Runtime::set_grant_sink`]).
pub type GrantSink = Arc<dyn Fn(&Value) + Send + Sync>;

/// The grant running now (one at a time).
pub(crate) struct Grant {
    answer: Value,
    cancel: Arc<AtomicBool>,
}

fn coded(code: &str, message: impl Into<String>) -> anyhow::Error {
    crate::issues::coded(code, message, None)
}

/// What `gh auth status` says of the active github.com account.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct GhStatus {
    pub account: Option<String>,
    /// `None`: gh lists no scopes (fine-grained token, `GH_TOKEN`).
    pub scopes: Option<Vec<String>>,
}

/// Parses `gh auth status` (gh-notes §2): the github.com block, its active account, its `Token scopes:` line.
pub(crate) fn parse_gh_status(text: &str) -> GhStatus {
    // The github.com block (one per host; a host line is not indented).
    let lines: Vec<&str> = text.lines().collect();
    let host = lines.iter().position(|l| !l.starts_with(char::is_whitespace) && l.trim() == "github.com");
    let block: Vec<&str> = match host {
        Some(i) => lines[i + 1..]
            .iter()
            .take_while(|l| l.trim().is_empty() || l.starts_with(char::is_whitespace))
            .copied()
            .collect(),
        // No github.com block: another host's block is never read (its scopes are not github.com's).
        None if lines.iter().any(|l| !l.trim().is_empty() && !l.starts_with(char::is_whitespace)) => vec![],
        None => lines,
    };
    // One account per `Logged in to` line (or a failed `… log in to` one, which names no account): the
    // active one, else the first.
    let mut accounts: Vec<Vec<&str>> = vec![];
    for l in block {
        if l.contains("Logged in to") || l.contains("log in to") || accounts.is_empty() {
            accounts.push(vec![]);
        }
        accounts.last_mut().expect("pushed").push(l);
    }
    let account = accounts
        .iter()
        .find(|a| a.iter().any(|l| l.contains("Active account: true")))
        .or_else(|| accounts.iter().find(|a| a.iter().any(|l| l.contains("Logged in to"))))
        .cloned()
        .unwrap_or_default();
    let scopes = account.iter().find_map(|l| l.split_once("Token scopes:")).map(|(_, rest)| {
        rest.split(',')
            .map(|s| s.trim().trim_matches(|c| c == '\'' || c == '"').trim().to_string())
            .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("none"))
            .collect::<Vec<_>>()
    });
    GhStatus { account: crate::integrations::gh_account(&account.join("\n")), scopes: scopes.filter(|s| !s.is_empty()) }
}

/// `gh` of `tracker_gh_access` for a failed `gh auth status`: `missing`, `offline` or `logged_out`.
fn gh_state_of_failure(text: &str) -> &'static str {
    let l = text.to_ascii_lowercase();
    if orch_tasks::intake::is_gh_missing_text(&l) {
        "missing"
    } else if orch_tasks::intake::is_offline_text(&l) {
        "offline"
    } else {
        "logged_out"
    }
}

/// Whether `granted` covers scope `need` (`project` covers `read:project`, never the reverse).
fn covers(granted: &[String], need: &str) -> bool {
    granted.iter().any(|g| g == need || (need == "read:project" && g == "project"))
}

/// How a `gh auth refresh` that printed no code is reported (gh-notes §1), from its output.
fn start_failure(seen: &[String], exited: bool) -> anyhow::Error {
    let text = seen.join("\n");
    let l = text.to_ascii_lowercase();
    let last = seen.iter().rev().find(|s| !s.trim().is_empty()).map(|s| s.trim().to_string());
    if l.contains("gh_token") || l.contains("github_token") {
        coded("interactive_only", "gh uses a token from the environment (GH_TOKEN): run the command in a terminal")
    } else if l.contains("not logged in") || l.contains("gh auth login") {
        coded("gh_auth", "gh is not logged in: run `gh auth login`")
    } else if exited && let Some(last) = last {
        coded("failed", last)
    } else {
        coded("interactive_only", "gh did not print a one-time code: run the command in a terminal")
    }
}

/// The one-time code (`ABCD-1234`) in a line of gh's output.
fn device_code(line: &str) -> Option<String> {
    line.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-')).find_map(|w| {
        let (a, b) = w.split_once('-')?;
        let part = |p: &str| p.len() == 4 && p.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
        (part(a) && part(b)).then(|| w.to_string())
    })
}

/// The device URL in a line of gh's output.
fn device_url(line: &str) -> Option<String> {
    let at = line.find("https://")?;
    let url: String = line[at..].chars().take_while(|c| !c.is_whitespace()).collect();
    Some(url.trim_end_matches(['.', ',', ')']).to_string())
}

impl Runtime {
    /// `tracker_gh_access(project)` (api-contract §2).
    pub fn tracker_gh_access(&self, project: &str) -> Result<Value> {
        if self.db.project(project)?.is_none() {
            return Err(coded("not_found", format!("no project {project:?}")));
        }
        let t = self.db.tracker(project)?;
        let mut needed: Vec<&str> = vec![];
        if let Tracker::Github { project_board: Some(_), .. } = &t.tracker {
            needed.push("read:project");
            if t.write_back.enabled {
                needed.push("project");
            }
        }
        let (gh, status) = match self.cfg.net.gh_auth_status() {
            Ok(text) => ("ok", parse_gh_status(&text)),
            Err(e) => {
                let text = format!("{e:#}");
                // gh exits 1 when any account or host has a problem: the active github.com login can still be fine.
                let s = parse_gh_status(&text);
                if s.account.is_some() { ("ok", s) } else { (gh_state_of_failure(&text), GhStatus::default()) }
            }
        };
        let missing: Vec<&str> = match &status.scopes {
            Some(g) => needed.iter().copied().filter(|n| !covers(g, n)).collect(),
            _ => vec![],
        };
        let failed = self.wb_failed(project);
        Ok(json!({
            "gh": gh,
            "account": status.account,
            "needed": needed,
            "granted": status.scopes,
            "missing": missing,
            "failed_writes": failed.len(),
            "last_failure": crate::writeback::last_failure(&failed),
        }))
    }

    /// Sets who is told of every `gh.grant` outcome (the app's webview).
    pub fn set_grant_sink(&self, f: GrantSink) {
        *crate::intake::lock(&self.grant_sink) = Some(f);
    }

    /// `gh_grant_scope(scope)` (api-contract §3): starts gh's device flow and answers its code; a
    /// call while a grant runs answers the running one.
    pub fn gh_grant_scope(&self, scope: &str) -> Result<Value> {
        if !GRANTABLE.contains(&scope) {
            return Err(coded("bad_scope", format!("scope {scope:?} can't be granted here")));
        }
        // Held while gh starts: a second call waits for it, then answers it.
        let mut slot = crate::intake::lock(&self.gh_grant);
        if let Some(g) = slot.as_ref() {
            return Ok(g.answer.clone());
        }
        let GhRefresh { lines, mut child } = self.cfg.net.gh_auth_refresh(scope).map_err(|e| {
            let se = orch_tasks::intake::gh_error(&e);
            coded(if se.code == "gh_missing" { "gh_missing" } else { "failed" }, se.message)
        })?;
        let mut scan = Scan::default();
        let until = Instant::now() + CODE_WAIT;
        let mut exited = false;
        while scan.code.is_none() || scan.url.is_none() {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            match lines.recv_timeout(left.min(POLL)) {
                Ok(l) => scan.push(l),
                Err(RecvTimeoutError::Timeout) => {}
                // Both streams closed: only the exit is left to see.
                Err(RecvTimeoutError::Disconnected) => std::thread::sleep(POLL),
            }
            if matches!(child.try_wait(), Ok(Some(_))) {
                // (what gh printed before it exited may still be on its way through the reader threads)
                exited = true;
                drain_after_exit(&lines, |l| scan.push(l));
                break;
            }
        }
        let Scan { code, url, seen } = scan;
        // (a code without the URL line: github.com's device page is always the same)
        let url = url.or_else(|| code.as_ref().map(|_| DEVICE_URL.to_string()));
        let (Some(code), Some(url)) = (code, url) else {
            child.kill();
            return Err(start_failure(&seen, exited));
        };
        let Some(me) = self.me.upgrade() else {
            child.kill();
            return Err(coded("failed", "the core is stopping"));
        };
        let answer = json!({"state": "waiting", "code": code, "url": url, "scope": scope});
        let cancel = Arc::new(AtomicBool::new(false));
        *slot = Some(Grant { answer: answer.clone(), cancel: cancel.clone() });
        drop(slot);
        let scope = scope.to_string();
        std::thread::spawn(move || me.grant_wait(&scope, lines, child, &cancel));
        Ok(answer)
    }

    /// `gh_grant_cancel()`: kills the running grant (it posts `failed`, "cancelled").
    pub fn gh_grant_cancel(&self) -> Value {
        let running = crate::intake::lock(&self.gh_grant).as_ref().map(|g| g.cancel.store(true, Ordering::SeqCst));
        json!({"ok": true, "cancelled": running.is_some()})
    }

    /// Waits for the grant's gh to end (or the timeout, or a cancel), then posts `gh.grant`.
    fn grant_wait(
        &self,
        scope: &str,
        lines: std::sync::mpsc::Receiver<String>,
        mut child: Box<dyn RefreshChild>,
        cancel: &AtomicBool,
    ) {
        let until = Instant::now() + GRANT_TIMEOUT;
        let mut last: Option<String> = None;
        let keep = |last: &mut Option<String>, l: String| {
            if !l.trim().is_empty() {
                *last = Some(l.trim().to_string());
            }
        };
        let outcome = loop {
            while let Ok(l) = lines.try_recv() {
                keep(&mut last, l);
            }
            if cancel.load(Ordering::SeqCst) {
                child.kill();
                break Err("cancelled".to_string());
            }
            match child.try_wait() {
                Ok(Some(true)) => break Ok(()),
                Ok(Some(false)) => {
                    // (its last stderr line, printed just before it exited, may not be read yet)
                    drain_after_exit(&lines, |l| keep(&mut last, l));
                    break Err(last.clone().unwrap_or_else(|| "gh auth refresh failed".into()));
                }
                Err(e) => break Err(format!("{e:#}")),
                Ok(None) if Instant::now() >= until => {
                    child.kill();
                    break Err("the one-time code expired: try again".into());
                }
                Ok(None) => std::thread::sleep(POLL),
            }
        };
        *crate::intake::lock(&self.gh_grant) = None;
        let detail = match outcome {
            Ok(()) => {
                let mut retried = serde_json::Map::new();
                for p in self.db.projects(false).unwrap_or_default() {
                    // (with write-back off there is nothing to retry)
                    match self.db.tracker(&p.id) {
                        Ok(t) if t.write_back.enabled && matches!(t.tracker, Tracker::Github { .. }) => {}
                        _ => continue,
                    }
                    if let Ok(r) = self.writeback_retry_all(&p.id) {
                        retried.insert(p.id.clone(), json!({"ok": r["ok"], "failed": r["failed"]}));
                    }
                }
                json!({"state": "done", "scope": scope, "message": null, "retried": retried})
            }
            Err(m) => json!({"state": "failed", "scope": scope, "message": m, "retried": null}),
        };
        self.event("-", "gh.grant", detail.clone());
        let _ = self.db.audit(
            "core",
            None,
            "gh_grant",
            Some(scope),
            detail["state"].as_str().unwrap_or(""),
            detail.clone(),
        );
        let sink = crate::intake::lock(&self.grant_sink).clone();
        if let Some(f) = sink {
            f(&detail);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gh_status_scopes_and_account() {
        let classic = "github.com\n  ✓ Logged in to github.com account octocat (keyring)\n  - Active account: true\n  - Git operation protocol: https\n  - Token: gho_****\n  - Token scopes: 'gist', 'read:org', 'repo', 'workflow'\n";
        assert_eq!(
            parse_gh_status(classic),
            GhStatus {
                account: Some("octocat".into()),
                scopes: Some(vec!["gist".into(), "read:org".into(), "repo".into(), "workflow".into()])
            }
        );
        let old =
            "github.com\n  ✓ Logged in to github.com as octo (oauth_token)\n  ✓ Token scopes: gist, read:org, repo\n";
        assert_eq!(parse_gh_status(old).scopes.unwrap(), ["gist", "read:org", "repo"]);
        // No scopes line (fine-grained token): granted null.
        let fine = "github.com\n  ✓ Logged in to github.com account octocat (GH_TOKEN)\n  - Active account: true\n  - Token: github_pat_****\n";
        assert_eq!(parse_gh_status(fine), GhStatus { account: Some("octocat".into()), scopes: None });
        assert_eq!(parse_gh_status("github.com\n  - Token scopes: none\n").scopes, None);
        // Two accounts: the active one's scopes; another host's block is not read.
        let two = "github.com\n  ✓ Logged in to github.com account old (keyring)\n  - Active account: false\n  - Token scopes: 'repo'\n\n  ✓ Logged in to github.com account new (keyring)\n  - Active account: true\n  - Token scopes: 'repo', 'project'\nghe.acme.com\n  ✓ Logged in to ghe.acme.com account x (keyring)\n  - Active account: true\n  - Token scopes: 'admin'\n";
        assert_eq!(
            parse_gh_status(two),
            GhStatus { account: Some("new".into()), scopes: Some(vec!["repo".into(), "project".into()]) }
        );
        // Another host only: its scopes are not github.com's.
        let ghe = "ghe.acme.com\n  ✓ Logged in to ghe.acme.com account x (keyring)\n  - Active account: true\n  - Token scopes: 'project'\n";
        assert_eq!(parse_gh_status(ghe), GhStatus { account: None, scopes: None });
        // The active account failed (gh exits 1): it names no account, the inactive one is not taken for it.
        let failed = "github.com\n  ✓ Logged in to github.com account old (keyring)\n  - Active account: false\n  - Token scopes: 'repo'\n\n  X Failed to log in to github.com account new (keyring)\n  - Active account: true\n  - The token in keyring is invalid.\n";
        assert_eq!(parse_gh_status(failed), GhStatus { account: None, scopes: None });
        assert_eq!(
            gh_state_of_failure("You are not logged into any GitHub hosts. To log in, run: gh auth login"),
            "logged_out"
        );
        assert_eq!(
            gh_state_of_failure("X Timeout trying to log in to github.com account octocat (keyring)"),
            "offline"
        );
        assert_eq!(gh_state_of_failure("error connecting to api.github.com"), "offline");
        assert_eq!(gh_state_of_failure("gh: not installed"), "missing");
    }

    #[test]
    fn device_code_and_url_in_gh_lines() {
        assert_eq!(device_code("! First copy your one-time code: AB1D-12C4").as_deref(), Some("AB1D-12C4"));
        assert_eq!(device_code("- gh config set -h github.com git_protocol https"), None);
        assert_eq!(
            device_url("Open this URL to continue in your web browser: https://github.com/login/device").as_deref(),
            Some("https://github.com/login/device")
        );
        let e = |seen: &[&str], exited| {
            let s: Vec<String> = seen.iter().map(|s| s.to_string()).collect();
            serde_json::from_str::<Value>(&start_failure(&s, exited).to_string()).unwrap()["code"].clone()
        };
        assert_eq!(
            e(&["The value of the GH_TOKEN environment variable is being used for authentication."], true),
            "interactive_only"
        );
        assert_eq!(
            e(&["not logged in to any hosts. Use 'gh auth login' to authenticate with a host"], true),
            "gh_auth"
        );
        assert_eq!(e(&["boom"], true), "failed");
        assert_eq!(e(&[], true), "interactive_only");
        assert_eq!(e(&["waiting"], false), "interactive_only");
    }
}
