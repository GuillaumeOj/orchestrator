//! Design artifacts (FR-602, FR-603): what each hand-off points at, the
//! previews of exported images and opening them. Images are read from the
//! **integration branch** (`git show`), or from the designer's branch before
//! the lead merges it; never from another agent's worktree on disk.

use std::path::PathBuf;

use anyhow::{Result, anyhow, bail};
use orch_store::{Artifact, Feature};
use orch_team::artifact::{ArtifactMeta, image_mime, parse};
use serde_json::{Value, json};

use crate::runtime::Runtime;

/// FR-603: the largest image a preview returns.
pub const MAX_PREVIEW_BYTES: u64 = 5 * 1024 * 1024;

/// The error for an image over `MAX_PREVIEW_BYTES`.
pub fn too_large(bytes: u64) -> anyhow::Error {
    anyhow!("too large ({:.1} MB)", bytes as f64 / (1024.0 * 1024.0))
}

/// Standard base64 (RFC 4648, padded).
pub fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// What `artifact_open` does with an artifact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenTarget {
    /// In the browser (an http(s) link only).
    Url(String),
    /// In Quick Look.
    File(PathBuf),
    /// In the app that owns the file type (a `.figcraft` file: FigCraft).
    App(PathBuf),
}

/// The meta of a stored artifact (parsed now for rows from before P3).
pub fn meta_of(a: &Artifact) -> Option<ArtifactMeta> {
    a.meta.clone().or_else(|| parse(&a.uri))
}

/// The design hand-off message (spec §3.2): the repo paths of the images
/// `from` exported in this run, and how the recipient gets them.
pub fn design_handoff_body(uri: &str, note: &str, from: &str, lead: &str, exported: &[String]) -> String {
    let mut s = format!("design {uri}{}", if note.is_empty() { String::new() } else { format!(": {note}") });
    let link_only = parse(uri).is_none_or(|m| m.tool != "file");
    if link_only {
        s.push_str(&format!(
            "\nThis is a link only: agents can't open it. Ask {from} to export the frame as an image under the \
             feature's design/ folder and hand it off again."
        ));
    }
    if !exported.is_empty() {
        s.push_str(&format!("\nExported images from {from} in this feature:"));
        for p in exported {
            s.push_str(&format!("\n- {p}"));
        }
        s.push_str(&format!(
            "\nThey are on {from}'s branch until {lead} merges it into the integration branch; then call \
             mcp__orch__sync_integration and Read them in your worktree."
        ));
    }
    s
}

/// A blob that is a Git LFS pointer becomes the real content through
/// `git lfs smudge`; an error (no git-lfs, or it can't fetch) is "not found on branch".
fn smudge_lfs(repo: &std::path::Path, bytes: Vec<u8>) -> Result<Vec<u8>> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    if !bytes.starts_with(b"version https://git-lfs") {
        return Ok(bytes);
    }
    let not_found = || anyhow!("not found on branch");
    let mut child = Command::new("git")
        .args(["lfs", "smudge"])
        .current_dir(repo)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| not_found())?;
    let mut stdin = child.stdin.take().ok_or_else(not_found)?;
    // The pointer is tiny: written from a thread so a full pipe can't deadlock the read.
    let writer = std::thread::spawn(move || stdin.write_all(&bytes));
    let out = child.wait_with_output().map_err(|_| not_found())?;
    let _ = writer.join();
    if !out.status.success() {
        return Err(not_found());
    }
    Ok(out.stdout)
}

impl Runtime {
    fn feature_and_artifact(&self, feature_id: &str, artifact: u64) -> Result<(Feature, Artifact)> {
        let f = self.db.feature(feature_id)?.ok_or_else(|| anyhow!("no feature {feature_id}"))?;
        let a = self
            .db
            .artifacts(&f.id)?
            .into_iter()
            .find(|a| a.id == artifact)
            .ok_or_else(|| anyhow!("no artifact {artifact} in {}", f.slug))?;
        Ok((f, a))
    }

    fn project_repo(&self, f: &Feature) -> Result<PathBuf> {
        Ok(self.db.project(&f.project_id)?.ok_or_else(|| anyhow!("no project {}", f.project_id))?.path)
    }

    /// The feature's artifacts with their meta (`on_integration` for files).
    pub fn artifacts_json(&self, f: &Feature) -> Result<Vec<Value>> {
        let repo = self.project_repo(f).ok();
        let branch = f.integration_branch();
        Ok(self
            .db
            .artifacts(&f.id)?
            .into_iter()
            .map(|mut a| {
                a.meta = meta_of(&a).map(|mut m| {
                    if let (Some(p), Some(repo)) = (&m.path, &repo) {
                        m.on_integration =
                            Some(orch_git::git::git_ok(repo, &["cat-file", "-e", &format!("{branch}:{p}")]));
                    }
                    m
                });
                json!(a)
            })
            .collect())
    }

    /// Branches an image may be on, best first: the integration branch, then
    /// the sender's own branches (newest first).
    fn image_sources(&self, repo: &std::path::Path, f: &Feature, a: &Artifact) -> Vec<(String, &'static str)> {
        let mut v = vec![(f.integration_branch(), "integration")];
        let pattern = format!("refs/heads/orch/{}/{}-*", f.slug, a.from_node);
        if let Ok(out) =
            orch_git::git::git(repo, &["for-each-ref", "--sort=-committerdate", "--format=%(refname:short)", &pattern])
        {
            v.extend(out.lines().filter(|l| !l.is_empty()).map(|l| (l.to_string(), "branch")));
        }
        v
    }

    /// FR-603: an exported image's bytes, from the integration branch, else
    /// the designer's branch. Errors: "not previewable", "too large (<size> MB)", "not
    /// found on branch".
    pub fn artifact_preview(&self, feature_id: &str, artifact: u64) -> Result<Value> {
        let (f, a) = self.feature_and_artifact(feature_id, artifact)?;
        let m = meta_of(&a).filter(|m| m.previewable).ok_or_else(|| anyhow!("not previewable"))?;
        let path = m.path.as_deref().ok_or_else(|| anyhow!("not previewable"))?;
        let mime = image_mime(path).ok_or_else(|| anyhow!("not previewable"))?;
        let repo = self.project_repo(&f)?;
        for (rev, source) in self.image_sources(&repo, &f, &a) {
            let spec = format!("{rev}:{path}");
            let Ok(size) = orch_git::git::git(&repo, &["cat-file", "-s", &spec]) else { continue };
            let size: u64 = size.trim().parse().unwrap_or(u64::MAX);
            if size > MAX_PREVIEW_BYTES {
                return Err(too_large(size));
            }
            let bytes = orch_git::git::git_bytes(&repo, &["cat-file", "blob", &spec])?;
            return Ok(json!({"mime": mime, "data_b64": base64(&bytes), "bytes": bytes.len(), "source": source}));
        }
        bail!("not found on branch")
    }

    /// Where `artifact_open` sends an artifact: a link to the browser, a file
    /// to Quick Look (the integration worktree's copy, else one extracted
    /// from the designer's branch into the app's folder).
    pub fn artifact_open_target(&self, feature_id: &str, artifact: u64) -> Result<OpenTarget> {
        let (f, a) = self.feature_and_artifact(feature_id, artifact)?;
        let m = meta_of(&a).ok_or_else(|| anyhow!("can't open {}", a.uri))?;
        let figcraft = m.tool == "figcraft";
        let wrap = |p: PathBuf| if figcraft { OpenTarget::App(p) } else { OpenTarget::File(p) };
        let Some(path) = m.path.filter(|_| m.tool == "file" || figcraft) else {
            let url = a.uri.trim();
            if !(url.starts_with("https://") || url.starts_with("http://")) {
                bail!("can't open {url}");
            }
            return Ok(OpenTarget::Url(url.to_string()));
        };
        // A FigCraft file is never opened in place: FigCraft saving into the integration
        // worktree would dirty it. It always opens a copy extracted from git.
        if let Some(wt) = f.integration_worktree.as_ref().filter(|_| !figcraft) {
            let p = wt.join(&path);
            // A plain file only: a committed symlink would open its target; the
            // git blob below is the link's own content.
            if std::fs::symlink_metadata(&p).is_ok_and(|m| m.file_type().is_file()) {
                return Ok(wrap(p));
            }
        }
        let repo = self.project_repo(&f)?;
        for (rev, _) in self.image_sources(&repo, &f, &a) {
            let Ok(bytes) = orch_git::git::git_bytes(&repo, &["cat-file", "blob", &format!("{rev}:{path}")]) else {
                continue;
            };
            // An LFS pointer that can't be fetched: try the next branch, then "not found".
            let Ok(bytes) = smudge_lfs(&repo, bytes) else { continue };
            let dir = self.cfg.paths.root.join("previews");
            std::fs::create_dir_all(&dir)?;
            let name = path.rsplit('/').next().unwrap_or("artifact");
            let out = dir.join(format!("{}-{name}", a.id));
            std::fs::write(&out, bytes)?;
            return Ok(wrap(out));
        }
        bail!("not found on branch")
    }

    /// Opens an artifact for the user (browser or Quick Look); returns at once.
    pub fn artifact_open(&self, feature_id: &str, artifact: u64) -> Result<()> {
        let target = self.artifact_open_target(feature_id, artifact)?;
        let in_app = matches!(target, OpenTarget::App(_));
        let mut cmd = match target {
            OpenTarget::Url(u) => {
                let mut c = std::process::Command::new("/usr/bin/open");
                c.arg(u);
                c
            }
            OpenTarget::File(p) => {
                let mut c = std::process::Command::new("/usr/bin/qlmanage");
                c.arg("-p").arg(p);
                c
            }
            OpenTarget::App(p) => {
                let mut c = std::process::Command::new("/usr/bin/open");
                c.arg(p);
                c
            }
        };
        cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null());
        if in_app {
            // `open` returns at once, and fails at once when no app owns the file type: say so.
            let out = cmd.stderr(std::process::Stdio::piped()).output()?;
            if !out.status.success() {
                bail!("can't open the file: {}", String::from_utf8_lossy(&out.stderr).trim());
            }
            return Ok(());
        }
        let mut child = cmd.stderr(std::process::Stdio::null()).spawn()?;
        // Quick Look stays up until the user closes it: reap it off the caller's thread.
        std::thread::spawn(move || child.wait());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::runtime::tests::test_runtime;
    use orch_git::fixtures::{commit_file, git as tgit, scratch_repo};

    fn add(rt: &Runtime, uri: &str, from: &str) -> u64 {
        rt.db
            .add_artifact(&Artifact {
                id: 0,
                feature_id: "f1".into(),
                team_run_id: Some("r1".into()),
                kind: "design".into(),
                uri: uri.into(),
                from_node: from.into(),
                to_node: Some("front".into()),
                note: String::new(),
                created_at_ms: 1,
                meta: parse(uri),
            })
            .unwrap()
    }

    #[test]
    fn previews_from_integration_then_the_designers_branch() {
        let (_d, rt) = test_runtime(true);
        let (_r, repo) = scratch_repo();
        let p = orch_git::project::register(&rt.db, &repo).unwrap();
        let f = Feature {
            id: "f1".into(),
            project_id: p.id.clone(),
            slug: "shop".into(),
            title: "Shop".into(),
            brief: String::new(),
            base: "main".into(),
            integration_worktree: None,
            allocation: None,
            created_at_ms: 1,
            archived: false,
        };
        rt.db.upsert_feature(&f).unwrap();
        let dir = ".orchestrator/features/shop/design";
        // ui exported two frames on its branch; only a.png is merged.
        tgit(&repo, &["checkout", "-q", "-b", "orch/shop/ui-1"]);
        std::fs::create_dir_all(repo.join(dir)).unwrap();
        commit_file(&repo, &format!("{dir}/a.png"), "PNGDATA", "a");
        tgit(&repo, &["checkout", "-q", "-b", "orch/shop/integration"]);
        tgit(&repo, &["checkout", "-q", "orch/shop/ui-1"]);
        commit_file(&repo, &format!("{dir}/b.svg"), "<svg/>", "b");
        commit_file(&repo, &format!("{dir}/a.figcraft"), "FC", "fc");
        std::fs::write(repo.join(format!("{dir}/big.png")), vec![0u8; MAX_PREVIEW_BYTES as usize + 1]).unwrap();
        tgit(&repo, &["add", "."]);
        tgit(&repo, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "big"]);
        tgit(&repo, &["checkout", "-q", "main"]);

        let a = add(&rt, &format!("{dir}/a.png"), "ui");
        let b = add(&rt, &format!("{dir}/b.svg"), "ui");
        let big = add(&rt, &format!("{dir}/big.png"), "ui");
        let gone = add(&rt, &format!("{dir}/gone.png"), "ui");
        let link = add(&rt, "https://design.penpot.app/#/workspace?file-id=f1&page-id=p1", "ui");
        let doc = add(&rt, ".orchestrator/features/shop/spec.md", "ux");
        let fc = add(&rt, &format!("{dir}/a.figcraft#1:2"), "ui");
        let fc_gone = add(&rt, &format!("{dir}/gone.figcraft"), "ui");
        let fc_lfs = add(&rt, &format!("{dir}/lfs.figcraft"), "ui");

        let v = rt.artifacts_json(&f).unwrap();
        let on = |i: usize| v[i]["meta"]["on_integration"].clone();
        assert_eq!((on(0), on(1)), (json!(true), json!(false)));
        assert_eq!(v[4]["meta"]["tool"], "penpot");
        assert_eq!(v[4]["meta"]["on_integration"], Value::Null);

        let pa = rt.artifact_preview("f1", a).unwrap();
        assert_eq!(
            (pa["mime"].as_str(), pa["source"].as_str(), pa["bytes"].as_u64()),
            (Some("image/png"), Some("integration"), Some(7))
        );
        assert_eq!(pa["data_b64"], base64(b"PNGDATA"));
        let pb = rt.artifact_preview("f1", b).unwrap();
        assert_eq!((pb["mime"].as_str(), pb["source"].as_str()), (Some("image/svg+xml"), Some("branch")));
        let err = |id| rt.artifact_preview("f1", id).unwrap_err().to_string();
        assert_eq!(err(big), "too large (5.0 MB)");
        assert_eq!(err(gone), "not found on branch");
        assert_eq!(err(link), "not previewable");
        assert_eq!(err(doc), "not previewable");
        assert!(rt.artifact_preview("f1", 999).is_err());

        // Open: links to the browser; files from the branch, extracted for Quick Look.
        assert_eq!(
            rt.artifact_open_target("f1", link).unwrap(),
            OpenTarget::Url("https://design.penpot.app/#/workspace?file-id=f1&page-id=p1".into())
        );
        let OpenTarget::File(p) = rt.artifact_open_target("f1", b).unwrap() else { panic!() };
        assert_eq!(std::fs::read_to_string(p).unwrap(), "<svg/>");
        // With an integration worktree holding the file, that copy is used.
        let wt = rt.cfg.paths.root.join("int");
        std::fs::create_dir_all(wt.join(dir)).unwrap();
        std::fs::write(wt.join(format!("{dir}/a.png")), "x").unwrap();
        rt.db.upsert_feature(&Feature { integration_worktree: Some(wt.clone()), ..f }).unwrap();
        assert_eq!(rt.artifact_open_target("f1", a).unwrap(), OpenTarget::File(wt.join(format!("{dir}/a.png"))));
        // A FigCraft file opens in its app, from a copy extracted from git, never the worktree's own.
        std::fs::write(wt.join(format!("{dir}/a.figcraft")), "fc").unwrap();
        let OpenTarget::App(p) = rt.artifact_open_target("f1", fc).unwrap() else { panic!() };
        assert!(p.starts_with(rt.cfg.paths.root.join("previews")), "{}", p.display());
        assert_eq!(rt.artifact_open_target("f1", fc_gone).unwrap_err().to_string(), "not found on branch");
        let v = rt.artifacts_json(&rt.db.feature("f1").unwrap().unwrap()).unwrap();
        assert_eq!((v[6]["meta"]["tool"].as_str(), v[6]["meta"]["node_id"].as_str()), (Some("figcraft"), Some("1:2")));
        assert_eq!(v[6]["meta"]["on_integration"], json!(false));
        // An LFS pointer that can't be smudged is not a file.
        tgit(&repo, &["checkout", "-q", "orch/shop/ui-1"]);
        commit_file(
            &repo,
            &format!("{dir}/lfs.figcraft"),
            "version https://git-lfs.github.com/spec/v1\noid sha256:0000000000000000000000000000000000000000000000000000000000000000\nsize 5\n",
            "lfs",
        );
        tgit(&repo, &["checkout", "-q", "main"]);
        assert_eq!(rt.artifact_open_target("f1", fc_lfs).unwrap_err().to_string(), "not found on branch");
        assert_eq!(smudge_lfs(&repo, b"plain".to_vec()).unwrap(), b"plain");
        // A symlink there is not followed: the blob from the branch is used instead.
        let secret = rt.cfg.paths.root.join("secret.txt");
        std::fs::write(&secret, "private").unwrap();
        std::os::unix::fs::symlink(&secret, wt.join(format!("{dir}/b.svg"))).unwrap();
        let OpenTarget::File(p) = rt.artifact_open_target("f1", b).unwrap() else { panic!() };
        assert!(p.starts_with(rt.cfg.paths.root.join("previews")), "{}", p.display());
        assert_eq!(std::fs::read_to_string(p).unwrap(), "<svg/>");
    }

    #[test]
    fn base64_rfc4648() {
        for (i, o) in
            [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE=")]
        {
            assert_eq!(base64(i.as_bytes()), o);
        }
        assert_eq!(base64(&[0xff, 0xfe, 0x00]), "//4A");
    }

    #[test]
    fn design_messages_list_exports_and_flag_links() {
        let b = design_handoff_body(
            ".orchestrator/features/f/design/a.png",
            "checkout",
            "ui",
            "lead",
            &[".orchestrator/features/f/design/a.png".into(), ".orchestrator/features/f/design/b.svg".into()],
        );
        assert!(b.starts_with("design .orchestrator/features/f/design/a.png: checkout"));
        assert!(b.contains("\n- .orchestrator/features/f/design/b.svg"));
        assert!(b.contains("mcp__orch__sync_integration") && !b.contains("link only"));
        let l = design_handoff_body("https://design.penpot.app/#/workspace?file-id=x", "", "ui", "lead", &[]);
        assert!(l.contains("link only") && l.contains("Ask ui to export"));
    }
}
