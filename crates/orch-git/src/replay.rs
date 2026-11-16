//! Replaying commits onto another base with plumbing only (feature
//! allow-multiple-prs §3.3): `merge-tree --write-tree` and `commit-tree`
//! in the object database. No worktree, no index, no hooks, no push.

use std::path::Path;

use anyhow::{Result, bail};

use crate::git::{git, git_env, git_output};

/// A commit as the replay reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitInfo {
    pub sha: String,
    /// The first parent (`None` for a root commit).
    pub parent: Option<String>,
    pub subject: String,
    /// The whole message (`%B`).
    pub message: String,
    pub author: Ident,
    /// The trailers as git parses them (`%(trailers)`), in order.
    pub trailers: Vec<(String, String)>,
}

impl CommitInfo {
    /// The first value of trailer `key` (case-insensitive key, as git).
    pub fn trailer(&self, key: &str) -> Option<&str> {
        self.trailers.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.as_str())
    }
}

/// A name, email and raw date (`<secs> <tz>`).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Ident {
    pub name: String,
    pub email: String,
    pub date: String,
}

const FS: char = '\x1f';
const RS: char = '\x1e';
const TS: char = '\x1d';

/// The first-parent commits of `from..to`, oldest first.
pub fn first_parent_commits(dir: &Path, from: &str, to: &str) -> Result<Vec<CommitInfo>> {
    let format = "--format=%H%x1f%P%x1f%an%x1f%ae%x1f%ad%x1f%s%x1f%(trailers:only,unfold,separator=%x1d)%x1f%B%x1e";
    let out = git(dir, &["log", "--first-parent", "--reverse", "--date=raw", format, &format!("{from}..{to}"), "--"])?;
    Ok(out.split(RS).map(|r| r.trim_start_matches('\n')).filter(|r| !r.is_empty()).filter_map(parse_commit).collect())
}

fn parse_commit(rec: &str) -> Option<CommitInfo> {
    let f: Vec<&str> = rec.splitn(8, FS).collect();
    let [sha, parents, name, email, date, subject, trailers, message] = f[..] else { return None };
    Some(CommitInfo {
        sha: sha.into(),
        parent: parents.split_whitespace().next().map(str::to_string),
        subject: subject.into(),
        message: message.into(),
        author: Ident { name: name.into(), email: email.into(), date: date.into() },
        trailers: trailers
            .split(TS)
            .filter_map(|t| t.split_once(':'))
            .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
            .filter(|(k, _)| !k.is_empty())
            .collect(),
    })
}

/// What `git merge-tree --write-tree` gave.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MergeTree {
    Clean(String),
    /// The conflicted paths.
    Conflict(Vec<String>),
}

/// The three-way merge of `ours` and `theirs` from `base`, in the object
/// database only.
pub fn merge_tree(dir: &Path, base: &str, ours: &str, theirs: &str) -> Result<MergeTree> {
    let args =
        ["merge-tree", "--write-tree", "--name-only", "--no-messages", &format!("--merge-base={base}"), ours, theirs];
    let out = git_output(dir, &args)?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut lines = stdout.lines();
    match out.status.code() {
        Some(0) => Ok(MergeTree::Clean(lines.next().unwrap_or("").trim().to_string())),
        Some(1) => {
            lines.next(); // the (partial) tree
            let mut files: Vec<String> = lines.map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect();
            files.dedup();
            Ok(MergeTree::Conflict(files))
        }
        _ => bail!("git merge-tree in {}: {}", dir.display(), String::from_utf8_lossy(&out.stderr).trim()),
    }
}

/// The tree of a commit.
pub fn tree_of(dir: &Path, rev: &str) -> Result<String> {
    Ok(git(dir, &["rev-parse", &format!("{rev}^{{tree}}")])?.trim().to_string())
}

/// A commit of `tree` on `parent`, with `author` as author and committer
/// (a rebuild of the same commits gives the same shas).
pub fn commit_tree(dir: &Path, tree: &str, parent: &str, message: &str, author: &Ident) -> Result<String> {
    let env = [
        ("GIT_AUTHOR_NAME", author.name.as_str()),
        ("GIT_AUTHOR_EMAIL", author.email.as_str()),
        ("GIT_AUTHOR_DATE", author.date.as_str()),
        ("GIT_COMMITTER_NAME", author.name.as_str()),
        ("GIT_COMMITTER_EMAIL", author.email.as_str()),
        ("GIT_COMMITTER_DATE", author.date.as_str()),
    ];
    let msg = message.trim_end();
    Ok(git_env(dir, &["commit-tree", "--no-gpg-sign", tree, "-p", parent, "-m", msg], &env)?.trim().to_string())
}

/// Points `refs/heads/<branch>` at `sha` (the branch is never checked out).
pub fn set_branch(dir: &Path, branch: &str, sha: &str) -> Result<()> {
    if branch.starts_with('-') || sha.starts_with('-') {
        bail!("invalid branch or commit");
    }
    git(dir, &["update-ref", &format!("refs/heads/{branch}"), sha])?;
    Ok(())
}

/// The outcome of [`replay`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Replay {
    Done {
        head: String,
        /// The commits replayed (their source shas).
        replayed: Vec<String>,
        /// Their change was already there: left out.
        skipped: Vec<String>,
    },
    Conflict {
        commit: String,
        files: Vec<String>,
    },
}

/// Applies each commit's own change (`diff(C^1, C)`) onto `onto`, in order,
/// keeping its author and message. An empty result is skipped. Nothing is
/// written but objects; a conflict stops at its commit.
pub fn replay(dir: &Path, onto: &str, commits: &[CommitInfo]) -> Result<Replay> {
    let mut head = onto.to_string();
    let mut tree = tree_of(dir, onto)?;
    let (mut replayed, mut skipped) = (vec![], vec![]);
    for c in commits {
        let Some(parent) = &c.parent else { bail!("{} is a root commit: it can't be replayed", c.sha) };
        match merge_tree(dir, parent, &head, &c.sha)? {
            MergeTree::Conflict(files) => return Ok(Replay::Conflict { commit: c.sha.clone(), files }),
            MergeTree::Clean(t) if t == tree => skipped.push(c.sha.clone()),
            MergeTree::Clean(t) => {
                head = commit_tree(dir, &t, &head, &c.message, &c.author)?;
                tree = t;
                replayed.push(c.sha.clone());
            }
        }
    }
    Ok(Replay::Done { head, replayed, skipped })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{commit_file, git as g, scratch_repo};

    fn sha(dir: &Path, rev: &str) -> String {
        g(dir, &["rev-parse", rev]).trim().to_string()
    }

    /// main + a feature branch with: a.txt, a merge of a side branch adding
    /// b.txt (trailer Orch-Task: 7), c.txt with Orch-Issue: #2.
    fn repo() -> (tempfile::TempDir, std::path::PathBuf, String) {
        let (d, repo) = scratch_repo();
        let fork = sha(&repo, "HEAD");
        g(&repo, &["checkout", "-q", "-b", "integ"]);
        commit_file(&repo, "a.txt", "a\n", "add a");
        g(&repo, &["checkout", "-q", "-b", "side", &fork]);
        commit_file(&repo, "b.txt", "b\n", "add b");
        g(&repo, &["checkout", "-q", "integ"]);
        g(
            &repo,
            &[
                "-c",
                "user.name=L",
                "-c",
                "user.email=l@x",
                "merge",
                "-q",
                "--no-ff",
                "-m",
                "Merge side\n\nOrch-Task: 7",
                "side",
            ],
        );
        commit_file(&repo, "c.txt", "c\n", "add c\n\nOrch-Issue: #2");
        (d, repo, fork)
    }

    #[test]
    fn first_parent_commits_read_parents_trailers_and_authors() {
        let (_d, repo, fork) = repo();
        let cs = first_parent_commits(&repo, &fork, "integ").unwrap();
        let subjects: Vec<_> = cs.iter().map(|c| c.subject.as_str()).collect();
        assert_eq!(subjects, ["add a", "Merge side", "add c"]);
        assert_eq!((cs[1].trailer("orch-task"), cs[2].trailer("Orch-Issue")), (Some("7"), Some("#2")));
        assert_eq!(cs[0].trailer("Orch-Task"), None);
        assert_eq!(cs[1].parent.as_deref(), Some(cs[0].sha.as_str()));
        assert_eq!((cs[1].author.name.as_str(), cs[1].author.email.as_str()), ("L", "l@x"));
        assert!(cs[1].message.starts_with("Merge side\n\nOrch-Task: 7"));
    }

    #[test]
    fn replay_applies_each_change_keeps_authors_and_skips_empty_ones() {
        let (_d, repo, fork) = repo();
        let cs = first_parent_commits(&repo, &fork, "integ").unwrap();
        // Only the merge and c: a.txt is left out.
        let Replay::Done { head, replayed, skipped } = replay(&repo, &fork, &[cs[1].clone(), cs[2].clone()]).unwrap()
        else {
            panic!("conflict")
        };
        assert_eq!((replayed.len(), skipped.len()), (2, 0));
        let files = g(&repo, &["ls-tree", "--name-only", &head]);
        assert_eq!(files.lines().collect::<Vec<_>>(), ["README.md", "b.txt", "c.txt"]);
        let log = g(&repo, &["log", "--format=%an|%s", &format!("{fork}..{head}")]);
        assert_eq!(log.lines().collect::<Vec<_>>(), ["T|add c", "L|Merge side"]);
        assert_eq!(g(&repo, &["rev-list", "--count", &format!("{fork}..{head}")]).trim(), "2", "linear");
        // Deterministic: the same replay gives the same head.
        let Replay::Done { head: again, .. } = replay(&repo, &fork, &[cs[1].clone(), cs[2].clone()]).unwrap() else {
            panic!()
        };
        assert_eq!(again, head);
        // Onto a base that already has the change: skipped.
        let Replay::Done { head: h2, replayed, skipped } = replay(&repo, &head, &[cs[2].clone()]).unwrap() else {
            panic!()
        };
        assert_eq!((h2, replayed.len(), skipped), (head.clone(), 0, vec![cs[2].sha.clone()]));
        set_branch(&repo, "orch/f/issue/2", &head).unwrap();
        assert_eq!(sha(&repo, "orch/f/issue/2"), head);
        assert_eq!(sha(&repo, "HEAD"), sha(&repo, "integ"), "nothing checked out moved");
    }

    #[test]
    fn a_change_on_a_file_another_commit_created_conflicts() {
        let (_d, repo) = scratch_repo();
        let fork = sha(&repo, "HEAD");
        commit_file(&repo, "x.txt", "one\n", "create x");
        commit_file(&repo, "x.txt", "two\n", "edit x");
        let cs = first_parent_commits(&repo, &fork, "HEAD").unwrap();
        let r = replay(&repo, &fork, &cs[1..]).unwrap();
        assert_eq!(r, Replay::Conflict { commit: cs[1].sha.clone(), files: vec!["x.txt".into()] });
        assert!(matches!(merge_tree(&repo, &fork, &fork, &cs[0].sha).unwrap(), MergeTree::Clean(_)));
    }
}
