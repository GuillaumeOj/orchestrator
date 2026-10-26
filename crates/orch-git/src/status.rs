//! Repository status (FR-102): branch, upstream, ahead/behind, staged /
//! unstaged / untracked / conflicted counts, last commit, stash count.

use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::git::git;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LastCommit {
    pub sha: String,
    pub summary: String,
    pub author: String,
    pub time_s: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RepoStatus {
    /// `None` when HEAD is detached.
    pub branch: Option<String>,
    pub head: Option<String>,
    pub upstream: Option<String>,
    /// `None` without an upstream.
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
    pub staged: u32,
    pub unstaged: u32,
    /// Tracked files with a staged or an unstaged change (each file once).
    pub changed: u32,
    pub untracked: u32,
    pub conflicted: u32,
    pub stash: u32,
    pub last_commit: Option<LastCommit>,
}

impl RepoStatus {
    /// Files with any change (the "dirty count" of J1).
    pub fn dirty(&self) -> u32 {
        self.changed + self.untracked + self.conflicted
    }
}

/// Parses `git status --porcelain=v2 --branch --show-stash`.
pub fn parse_porcelain_v2(out: &str) -> RepoStatus {
    let mut s = RepoStatus::default();
    for line in out.lines() {
        if let Some(h) = line.strip_prefix("# ") {
            let (k, v) = h.split_once(' ').unwrap_or((h, ""));
            match k {
                "branch.oid" if v != "(initial)" => s.head = Some(v.into()),
                "branch.head" if v != "(detached)" => s.branch = Some(v.into()),
                "branch.upstream" => s.upstream = Some(v.into()),
                "branch.ab" => {
                    let mut it = v.split_whitespace();
                    s.ahead = it.next().and_then(|a| a.trim_start_matches('+').parse().ok());
                    s.behind = it.next().and_then(|b| b.trim_start_matches('-').parse().ok());
                }
                "stash" => s.stash = v.trim().parse().unwrap_or(0),
                _ => {}
            }
            continue;
        }
        let mut it = line.splitn(3, ' ');
        match it.next() {
            Some("1") | Some("2") => {
                s.changed += 1;
                let xy = it.next().unwrap_or("..").as_bytes();
                if xy.first().is_some_and(|c| *c != b'.') {
                    s.staged += 1;
                }
                if xy.get(1).is_some_and(|c| *c != b'.') {
                    s.unstaged += 1;
                }
            }
            Some("u") => s.conflicted += 1,
            Some("?") => s.untracked += 1,
            _ => {}
        }
    }
    s
}

pub fn status(repo: &Path) -> Result<RepoStatus> {
    let out = git(repo, &["status", "--porcelain=v2", "--branch", "--show-stash", "--untracked-files=normal"])?;
    let mut s = parse_porcelain_v2(&out);
    s.last_commit = last_commit(repo, "HEAD");
    Ok(s)
}

pub fn last_commit(repo: &Path, rev: &str) -> Option<LastCommit> {
    let out = git(repo, &["log", "-1", "--format=%H%x1f%s%x1f%an%x1f%ct", rev, "--"]).ok()?;
    let mut f = out.trim_end().split('\x1f');
    Some(LastCommit {
        sha: f.next()?.into(),
        summary: f.next()?.into(),
        author: f.next()?.into(),
        time_s: f.next()?.parse().ok()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{commit_file, git as tgit, scratch_repo};

    #[test]
    fn parses_v2() {
        let s = parse_porcelain_v2(
            "# branch.oid abc\n# branch.head main\n# branch.upstream origin/main\n# branch.ab +2 -3\n# stash 4\n\
             1 M. N... 100644 100644 100644 a b src/a.rs\n1 .M N... 100644 100644 100644 a b src/b.rs\n\
             1 MM N... 100644 100644 100644 a b src/c.rs\n2 R. N... 100644 100644 100644 a b R100 new\told\n\
             u UU N... 1 2 3 4 a b c x.rs\n? new.txt\n? other.txt\n! ignored\n",
        );
        assert_eq!(s.branch.as_deref(), Some("main"));
        assert_eq!((s.ahead, s.behind, s.stash), (Some(2), Some(3), 4));
        assert_eq!((s.staged, s.unstaged, s.untracked, s.conflicted), (3, 2, 2, 1));
        assert_eq!(s.dirty(), 4 + 2 + 1, "3 staged + 1 unstaged-only file, untracked, conflicted");
        let d = parse_porcelain_v2("# branch.oid (initial)\n# branch.head (detached)\n");
        assert_eq!((d.branch, d.head, d.ahead), (None, None, None));
    }

    #[test]
    fn real_repo_with_upstream_and_stash() {
        let (_d, repo) = scratch_repo();
        let remote = repo.parent().unwrap().join("remote.git");
        tgit(repo.parent().unwrap(), &["clone", "-q", "--bare", repo.to_str().unwrap(), remote.to_str().unwrap()]);
        tgit(&repo, &["remote", "add", "origin", remote.to_str().unwrap()]);
        tgit(&repo, &["fetch", "-q", "origin"]);
        tgit(&repo, &["branch", "-q", "--set-upstream-to=origin/main"]);
        commit_file(&repo, "a.txt", "1", "ahead one");
        std::fs::write(repo.join("README.md"), "changed").unwrap();
        tgit(&repo, &["stash", "-q"]);
        std::fs::write(repo.join("u.txt"), "x").unwrap();
        std::fs::write(repo.join("a.txt"), "2").unwrap();
        let s = status(&repo).unwrap();
        assert_eq!(s.branch.as_deref(), Some("main"));
        assert_eq!(s.upstream.as_deref(), Some("origin/main"));
        assert_eq!((s.ahead, s.behind), (Some(1), Some(0)));
        assert_eq!((s.unstaged, s.untracked, s.stash), (1, 1, 1));
        assert_eq!(s.dirty(), 2);
        assert_eq!(s.last_commit.unwrap().summary, "ahead one");
    }
}
