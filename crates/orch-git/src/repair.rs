//! Repo-move repair (J10, ADR-003). Worktrees use relative links, so moving
//! the repo together with its sibling folder needs nothing. When the links
//! break anyway (one moved without the other), the core runs a **targeted**
//! `git -C <repo> worktree repair <every known worktree path>` (a plain
//! `repair` from the main checkout can't find moved worktrees) and re-locks.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::git::{git, real};
use crate::worktree::{self, WorktreeRow};

/// Where a worktree's links point and whether they agree.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LinkCheck {
    pub path: PathBuf,
    /// The admin folder in `<repo>/.git/worktrees/<name>` the worktree points to.
    pub gitdir: Option<PathBuf>,
    pub ok: bool,
    pub problem: Option<String>,
}

fn resolve(base: &Path, target: &str) -> PathBuf {
    let t = Path::new(target.trim());
    if t.is_absolute() { t.to_path_buf() } else { base.join(t) }
}

/// Checks both links: `<wt>/.git` → admin dir, and the admin dir's `gitdir`
/// file → back to `<wt>/.git`.
pub fn check_links(wt: &Path) -> LinkCheck {
    let mut c = LinkCheck { path: wt.to_path_buf(), gitdir: None, ok: false, problem: None };
    let dotgit = wt.join(".git");
    let Ok(text) = std::fs::read_to_string(&dotgit) else {
        c.problem = Some(if wt.is_dir() { "no .git file" } else { "folder missing" }.into());
        return c;
    };
    let Some(target) = text.lines().find_map(|l| l.strip_prefix("gitdir:")) else {
        c.problem = Some("malformed .git file".into());
        return c;
    };
    let admin = resolve(wt, target);
    if !admin.is_dir() {
        c.problem = Some(format!("{} does not exist", admin.display()));
        return c;
    }
    c.gitdir = Some(real(&admin));
    let back = std::fs::read_to_string(admin.join("gitdir")).unwrap_or_default();
    let back = resolve(&admin, back.trim());
    if real(&back) != real(&dotgit) {
        c.problem = Some(format!("admin dir points to {}", back.display()));
        return c;
    }
    c.ok = true;
    c
}

/// Old → new paths after the repo moved from `old_repo` to `new_repo`. A
/// worktree under the old sibling folder maps under the new one when that
/// folder moved along (it exists), otherwise it stays where it is.
pub fn plan_relocation(old_repo: &Path, new_repo: &Path, rows: &[WorktreeRow]) -> Vec<(PathBuf, PathBuf)> {
    let old_root = worktree::worktree_root(old_repo);
    let new_root = worktree::worktree_root(new_repo);
    let mut map = vec![(old_repo.to_path_buf(), real(new_repo))];
    for r in rows {
        let new = match r.path.strip_prefix(&old_root) {
            Ok(rel) if new_root.join(rel).is_dir() => new_root.join(rel),
            _ => r.path.clone(),
        };
        map.push((r.path.clone(), new));
    }
    map
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RepairReport {
    pub repaired: Vec<PathBuf>,
    pub already_ok: Vec<PathBuf>,
    pub failed: Vec<(PathBuf, String)>,
    pub relocked: Vec<PathBuf>,
}

/// Repairs the links of the given worktrees (paths as they are now) and
/// re-locks them. Missing folders are reported, never created.
pub fn repair(repo: &Path, worktrees: &[(PathBuf, Option<String>)]) -> Result<RepairReport> {
    if !repo.join(".git").exists() {
        bail!("{} is not a main checkout", repo.display());
    }
    let mut rep = RepairReport::default();
    let broken: Vec<&PathBuf> = worktrees
        .iter()
        .map(|(p, _)| p)
        .filter(|p| {
            let ok = check_links(p).ok;
            if ok {
                rep.already_ok.push((*p).clone());
            }
            !ok && p.is_dir()
        })
        .collect();
    for (p, _) in worktrees {
        if !p.is_dir() {
            rep.failed.push((p.clone(), "folder missing".into()));
        }
    }
    if !broken.is_empty() {
        let mut args: Vec<String> =
            vec!["-c".into(), "worktree.useRelativePaths=true".into(), "worktree".into(), "repair".into()];
        args.extend(broken.iter().map(|p| p.display().to_string()));
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        let r = git(repo, &argv);
        for p in broken {
            let c = check_links(p);
            if c.ok {
                rep.repaired.push(p.clone());
            } else {
                let why = match &r {
                    Err(e) => format!("{e:#}"),
                    Ok(_) => c.problem.unwrap_or_default(),
                };
                rep.failed.push((p.clone(), why));
            }
        }
    }
    for (p, agent) in worktrees {
        if check_links(p).ok && worktree::lock(repo, p, agent.as_deref()).is_ok() {
            rep.relocked.push(p.clone());
        }
    }
    Ok(rep)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{MemLedger, scratch_repo};
    use orch_protocol::WorktreeKindSpec;

    fn setup() -> (tempfile::TempDir, PathBuf, Vec<WorktreeRow>) {
        let (d, repo) = scratch_repo();
        let led = MemLedger::default();
        let rows: Vec<WorktreeRow> = ["a", "b"]
            .iter()
            .map(|s| {
                worktree::ensure(&led, &repo, &WorktreeKindSpec::Solo { slug: (*s).into() }, None, Some(s)).unwrap()
            })
            .collect();
        (d, repo, rows)
    }

    #[test]
    fn joint_move_needs_nothing() {
        let (d, repo, rows) = setup();
        let new_parent = d.path().join("moved");
        std::fs::create_dir_all(&new_parent).unwrap();
        std::fs::rename(&repo, new_parent.join("demo")).unwrap();
        std::fs::rename(worktree::worktree_root(&repo), new_parent.join("demo.worktrees")).unwrap();
        let new_repo = real(&new_parent.join("demo"));
        let map = plan_relocation(&repo, &new_repo, &rows);
        for (_, new) in &map[1..] {
            assert!(check_links(new).ok, "{:?}", check_links(new));
        }
        assert!(git(&map[1].1, &["status", "--short"]).is_ok());
    }

    #[test]
    fn repo_moved_alone_is_repaired_and_relocked() {
        let (d, repo, rows) = setup();
        let new_repo = d.path().join("elsewhere/demo");
        std::fs::create_dir_all(new_repo.parent().unwrap()).unwrap();
        std::fs::rename(&repo, &new_repo).unwrap();
        let new_repo = real(&new_repo);
        let map = plan_relocation(&repo, &new_repo, &rows);
        assert_eq!(map[1].1, rows[0].path, "the sibling folder did not move: worktrees stay");
        assert!(!check_links(&rows[0].path).ok);
        // A missing worktree is reported, not created.
        let mut targets: Vec<(PathBuf, Option<String>)> =
            rows.iter().map(|r| (r.path.clone(), r.agent_id.clone())).collect();
        targets.push((d.path().join("nope"), None));
        let rep = repair(&new_repo, &targets).unwrap();
        assert_eq!(rep.repaired.len(), 2, "{rep:?}");
        assert_eq!(rep.failed.len(), 1);
        assert_eq!(rep.relocked.len(), 2);
        for r in &rows {
            assert!(check_links(&r.path).ok);
            let text = std::fs::read_to_string(r.path.join(".git")).unwrap();
            assert!(text.contains("gitdir: ../"), "still relative: {text}");
        }
        assert!(worktree::list(&new_repo).unwrap().iter().filter(|w| w.locked).count() == 2);
        // Idempotent.
        let again = repair(&new_repo, &targets[..2]).unwrap();
        assert_eq!(again.already_ok.len(), 2);
    }
}
