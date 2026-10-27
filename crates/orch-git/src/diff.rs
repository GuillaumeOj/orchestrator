//! Diffs per worktree (FR-107): against the base (or integration) branch from
//! their merge base, including uncommitted and untracked changes, with a
//! per-file list for the file tree's change markers.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::git::git;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    #[serde(default)]
    pub old_path: Option<String>,
    /// `A` `M` `D` `R` `C` `T` `U`, or `?` for untracked.
    pub status: String,
    /// `None` for binary files and untracked ones.
    pub added: Option<u32>,
    pub deleted: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Diff {
    /// What was compared against (`main`, `orch/x/integration`…).
    pub against: String,
    /// The merge base commit.
    pub base_sha: String,
    pub files: Vec<FileChange>,
    /// Unified patch, cut at `max_patch_bytes`.
    pub patch: String,
    pub truncated: bool,
}

fn parse_name_status_z(out: &str) -> Vec<(String, Option<String>, String)> {
    let mut v = vec![];
    let mut it = out.split('\0').filter(|s| !s.is_empty());
    while let Some(st) = it.next() {
        let code = st.chars().next().unwrap_or('M').to_string();
        if code == "R" || code == "C" {
            let (Some(old), Some(new)) = (it.next(), it.next()) else { break };
            v.push((new.to_string(), Some(old.to_string()), code));
        } else {
            let Some(p) = it.next() else { break };
            v.push((p.to_string(), None, code));
        }
    }
    v
}

fn parse_numstat_z(out: &str) -> BTreeMap<String, (Option<u32>, Option<u32>)> {
    let mut m = BTreeMap::new();
    let mut it = out.split('\0');
    while let Some(rec) = it.next() {
        if rec.is_empty() {
            continue;
        }
        let mut f = rec.splitn(3, '\t');
        let (a, d, p) = (f.next().unwrap_or("-"), f.next().unwrap_or("-"), f.next().unwrap_or(""));
        let path = if p.is_empty() {
            // Rename: the old then the new path follow as their own fields.
            let _old = it.next();
            it.next().unwrap_or("").to_string()
        } else {
            p.to_string()
        };
        m.insert(path, (a.parse().ok(), d.parse().ok()));
    }
    m
}

/// `against` is a branch or commit; the diff runs from its merge base with
/// HEAD to the working tree. `max_patch_bytes` bounds the patch in memory.
pub fn diff(worktree: &Path, against: &str, max_patch_bytes: usize) -> Result<Diff> {
    let base_sha = git(worktree, &["merge-base", against, "HEAD"])?.trim().to_string();
    let ns = git(worktree, &["diff", "--name-status", "-z", "-M", &base_sha])?;
    let num = parse_numstat_z(&git(worktree, &["diff", "--numstat", "-z", "-M", &base_sha])?);
    let mut files: Vec<FileChange> = parse_name_status_z(&ns)
        .into_iter()
        .map(|(path, old_path, status)| {
            let (added, deleted) = num.get(&path).copied().unwrap_or((None, None));
            FileChange { path, old_path, status, added, deleted }
        })
        .collect();
    for u in git(worktree, &["ls-files", "--others", "--exclude-standard", "-z"])?.split('\0').filter(|s| !s.is_empty())
    {
        files.push(FileChange { path: u.into(), old_path: None, status: "?".into(), added: None, deleted: None });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    // -z output is never quoted; the patch headers are, unless quotepath is off.
    let mut patch = git(worktree, &["-c", "core.quotepath=false", "diff", "-M", "--no-color", &base_sha])?;
    let truncated = patch.len() > max_patch_bytes;
    if truncated {
        let mut cut = max_patch_bytes;
        while !patch.is_char_boundary(cut) {
            cut -= 1;
        }
        patch.truncate(cut);
    }
    Ok(Diff { against: against.into(), base_sha, files, patch, truncated })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{commit_file, git as tgit, scratch_repo};

    #[test]
    fn diff_against_base_with_uncommitted_untracked_and_rename() {
        let (_d, repo) = scratch_repo();
        commit_file(&repo, "keep.txt", "a\nb\n", "base file");
        tgit(&repo, &["checkout", "-q", "-b", "orch/solo/x"]);
        commit_file(&repo, "new.rs", "fn main() {}\n", "add");
        tgit(&repo, &["mv", "keep.txt", "kept.txt"]);
        tgit(&repo, &["-c", "user.name=T", "-c", "user.email=t@x", "commit", "-qm", "rename"]);
        std::fs::write(repo.join("README.md"), "# demo\nmore\n").unwrap();
        std::fs::write(repo.join("scratch.txt"), "x").unwrap();
        // main moves on: the diff still starts at the merge base.
        tgit(&repo, &["checkout", "-q", "main"]);
        commit_file(&repo, "main-only.txt", "m", "main moves");
        tgit(&repo, &["checkout", "-q", "orch/solo/x"]);

        let d = diff(&repo, "main", 1 << 20).unwrap();
        let by: BTreeMap<_, _> = d.files.iter().map(|f| (f.path.as_str(), f)).collect();
        assert_eq!(by["new.rs"].status, "A");
        assert_eq!(by["new.rs"].added, Some(1));
        assert_eq!(by["kept.txt"].status, "R");
        assert_eq!(by["kept.txt"].old_path.as_deref(), Some("keep.txt"));
        assert_eq!(by["README.md"].status, "M");
        assert_eq!(by["scratch.txt"].status, "?");
        assert!(!by.contains_key("main-only.txt"));
        assert!(d.patch.contains("+more"));
        let small = diff(&repo, "main", 10).unwrap();
        assert!(small.truncated && small.patch.len() <= 10);
    }
}
