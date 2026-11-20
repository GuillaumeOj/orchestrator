//! Where a feature's docs and designs live (change-how-specs-are-saved §3),
//! and the confined reads of them (the spec gate, the Feature pane, the
//! Overwatch, Notion). Every reader goes through [`FeatureDocs::of`]: no
//! `.orchestrator/features/<slug>` anywhere else.

use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use orch_store::Feature;
use orch_team::{DocsLocation, Loc};
use serde_json::{Value, json};

use crate::team::cut;

pub struct FeatureDocs;

impl FeatureDocs {
    /// The folders `f` was created with; the legacy ones for a feature
    /// created before the setting.
    pub fn of(f: &Feature) -> DocsLocation {
        f.docs_location.clone().unwrap_or_else(|| DocsLocation::legacy(&f.slug))
    }
}

/// The docs folder of `f` (in its integration worktree for a repo folder);
/// `None` when it has no worktree and its docs are in the repo.
pub fn docs_dir_of(f: &Feature) -> Option<PathBuf> {
    let loc = FeatureDocs::of(f);
    match (&f.integration_worktree, loc.docs.is_app()) {
        (_, true) => Some(loc.docs.dir_in(Path::new("/"))),
        (Some(w), false) => Some(loc.docs.dir_in(w)),
        (None, false) => None,
    }
}

/// The worktree the docs of `f` are read against (an app folder needs none).
pub fn read_root(f: &Feature) -> Option<PathBuf> {
    f.integration_worktree.clone().or_else(|| FeatureDocs::of(f).docs.is_app().then(|| PathBuf::from("/")))
}

/// The full paths of `f`'s docs, spec.md first (the Overwatch's readers).
pub fn doc_paths_of(f: &Feature) -> Vec<String> {
    let (Some(w), Some(dir)) = (read_root(f), docs_dir_of(f)) else { return vec![] };
    feature_docs(&FeatureDocs::of(f), &w).into_iter().map(|n| dir.join(n).display().to_string()).collect()
}

/// `<app data>/projects/<project>/features/<slug>`: the app folders of a
/// feature whose docs or designs are not saved in the repository.
pub fn app_base(root: &Path, project_id: &str, slug: &str) -> PathBuf {
    root.join("projects").join(project_id).join("features").join(slug)
}

/// The feature's app folders as paths (none when everything is in the repo).
pub fn app_dirs(loc: &DocsLocation) -> Vec<PathBuf> {
    loc.app_dirs().into_iter().map(PathBuf::from).collect()
}

/// [`app_dirs`] of a feature's snapshot.
pub fn app_dirs_of(f: &Feature) -> Vec<PathBuf> {
    app_dirs(&FeatureDocs::of(f))
}

/// One folder of a feature, canonical. A repo folder (or a folder above it)
/// symlinked out of the worktree is refused, and so is an app folder that
/// is itself a symlink: every read below is confined to it.
fn real_dir(l: &Loc, worktree: &Path) -> Result<PathBuf> {
    let dir = l.dir_in(worktree);
    if l.is_app() {
        if std::fs::symlink_metadata(&dir).map_err(|_| anyhow!("not found"))?.file_type().is_symlink() {
            bail!("outside the feature folder");
        }
        return dir.canonicalize().map_err(|_| anyhow!("not found"));
    }
    let (Ok(dir), Ok(top)) = (dir.canonicalize(), worktree.canonicalize()) else { bail!("not found") };
    if !dir.starts_with(&top) {
        bail!("outside the feature folder");
    }
    Ok(dir)
}

/// The path of a doc as the gate shows it: repo-relative or absolute.
pub fn doc_path(loc: &DocsLocation, name: &str) -> String {
    format!("{}/{name}", loc.docs.dir.trim_end_matches('/'))
}

/// `[{path, lines}]` of the feature's docs (the spec gate's `docs` block).
pub fn doc_lines(loc: &DocsLocation, worktree: &Path, docs: &[String]) -> Value {
    // The names come from `feature_docs` (plain `.md` names in the folder).
    let dir = real_dir(&loc.docs, worktree);
    let files: Vec<Value> = docs
        .iter()
        .map(|d| {
            let lines = dir.as_ref().ok().and_then(|dir| std::fs::read_to_string(dir.join(d)).ok());
            let lines = lines.map_or(0, |t| t.lines().count());
            json!({"path": doc_path(loc, d), "lines": lines})
        })
        .collect();
    json!(files)
}

/// The markdown files in the feature's docs folder.
pub fn feature_docs(loc: &DocsLocation, worktree: &Path) -> Vec<String> {
    let mut v: Vec<String> = real_dir(&loc.docs, worktree)
        .and_then(|dir| Ok(std::fs::read_dir(dir)?))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "md"))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    v.sort_by_key(|n| (n != "spec.md", n.clone()));
    v
}

/// Reads one doc of a feature (for the gate preview and the Feature pane),
/// refusing anything outside the docs folder.
pub fn read_feature_doc(loc: &DocsLocation, worktree: &Path, name: &str) -> Result<String> {
    if name.contains('/') || name.contains("..") || !name.ends_with(".md") {
        bail!("invalid doc name {name:?}");
    }
    let p = real_dir(&loc.docs, worktree).with_context(|| format!("docs in {}", loc.docs.dir))?.join(name);
    let text = std::fs::read_to_string(&p).with_context(|| format!("read {}", p.display()))?;
    Ok(cut(&text, 200_000))
}

/// `base/rel` with `.` and `..` folded without touching the disk; `None`
/// when it climbs above the filesystem root.
fn lexical_join(base: &Path, rel: &Path) -> Option<PathBuf> {
    let mut out = base.to_path_buf();
    for c in rel.components() {
        match c {
            Component::Normal(n) => out.push(n),
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            _ => return None,
        }
    }
    Some(out)
}

/// An image a feature doc points at (`design/home.png`, or
/// `../../design/<slug>/home.png` for a separate designs folder), relative
/// to the docs folder, read as `{mime, data_b64, bytes}`. It must land in
/// the docs or the designs folder: refuses anything that leaves them
/// (absolute, `..` out of both, a symlink to outside them), a symlink as the
/// file itself, a non-image and a file over `MAX_PREVIEW_BYTES`.
pub fn read_feature_asset(loc: &DocsLocation, worktree: &Path, path: &str) -> Result<Value> {
    use crate::artifacts::base64;
    let not_found = || anyhow!("not found");
    let outside = || anyhow!("outside the feature folder");
    let rel = Path::new(path);
    if !rel.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir | Component::ParentDir)) {
        return Err(outside());
    }
    // On the path first: it must stay in one of the two folders as written.
    let (docs, designs) = (loc.docs.dir_in(worktree), loc.designs.dir_in(worktree));
    let p = lexical_join(&docs, rel).ok_or_else(outside)?;
    if !(p.starts_with(&docs) || p.starts_with(&designs)) {
        return Err(outside());
    }
    let mime = orch_team::artifact::doc_image_mime(path).ok_or_else(|| anyhow!("not an image"))?;
    // A folder linked out of its place confines nothing: refused, not skipped.
    let mut roots: Vec<PathBuf> = vec![];
    for l in [&loc.docs, &loc.designs] {
        match real_dir(l, worktree) {
            Ok(r) => roots.push(r),
            Err(e) if e.to_string() == "not found" => {}
            Err(e) => return Err(e),
        }
    }
    if roots.is_empty() {
        return Err(not_found());
    }
    if std::fs::symlink_metadata(&p).map_err(|_| not_found())?.file_type().is_symlink() {
        return Err(outside());
    }
    let real = p.canonicalize().map_err(|_| not_found())?;
    if !roots.iter().any(|r| real.starts_with(r)) {
        return Err(outside());
    }
    let bytes = read_capped(&real)?;
    Ok(json!({"mime": mime, "data_b64": base64(&bytes), "bytes": bytes.len()}))
}

/// A plain file's bytes, at most `MAX_PREVIEW_BYTES`. The callers' checks are
/// on the path; the read is on a handle that can't follow a symlink swapped
/// in as the file since (a directory above it swapped in between is not
/// caught), re-checked and capped (the folder is the agents' to change while
/// we read).
pub(crate) fn read_capped(real: &Path) -> Result<Vec<u8>> {
    use crate::artifacts::{MAX_PREVIEW_BYTES, too_large};
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let not_found = || anyhow!("not found");
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(real)
        .map_err(|_| not_found())?;
    let meta = file.metadata().map_err(|_| not_found())?;
    if !meta.is_file() {
        return Err(not_found());
    }
    if meta.len() > MAX_PREVIEW_BYTES {
        return Err(too_large(meta.len()));
    }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    file.take(MAX_PREVIEW_BYTES + 1).read_to_end(&mut bytes).map_err(|_| not_found())?;
    if bytes.len() as u64 > MAX_PREVIEW_BYTES {
        return Err(too_large(bytes.len() as u64));
    }
    Ok(bytes)
}

/// Is `p` (absolute) a regular file inside one of the feature's app folders?
/// For a design hand-off of a file that is not in the repository: a folder or
/// a symlink (even one that stays inside) is refused.
pub fn is_app_file(loc: &DocsLocation, p: &Path) -> bool {
    if !std::fs::symlink_metadata(p).is_ok_and(|m| m.is_file()) {
        return false;
    }
    let Ok(real) = p.canonicalize() else { return false };
    app_dirs(loc)
        .iter()
        .filter_map(|d| real_dir(&Loc::app(d.to_string_lossy()), Path::new("/")).ok())
        .any(|d| real.starts_with(d))
}

/// Creates the feature's app folders (launch, reveal).
pub fn create_app_dirs(loc: &DocsLocation) -> Result<()> {
    for l in [&loc.docs, &loc.designs].into_iter().filter(|l| l.is_app()) {
        std::fs::create_dir_all(&l.dir).with_context(|| format!("create {}", l.dir))?;
    }
    Ok(())
}

/// Removes the feature's app folder `<root>/projects/<project>/features/<slug>`
/// (deleting the feature). Only that folder, never through a symlink: a
/// folder that is a symlink, or resolves outside `<root>/projects`, is left.
pub fn remove_app_base(root: &Path, project_id: &str, slug: &str) -> Result<bool> {
    if !orch_git::valid_id(project_id) || !orch_git::valid_id(slug) {
        bail!("invalid feature folder");
    }
    let base = app_base(root, project_id, slug);
    let Ok(meta) = std::fs::symlink_metadata(&base) else { return Ok(false) };
    if meta.file_type().is_symlink() || !meta.is_dir() {
        bail!("{} is not a folder: left as it is", base.display());
    }
    let projects = root.join("projects").canonicalize().context("app projects folder")?;
    if projects != root.canonicalize()?.join("projects") {
        bail!("{} resolves outside the app's data: left as it is", base.display());
    }
    let real = base.canonicalize()?;
    if real != projects.join(project_id).join("features").join(slug) {
        bail!("{} resolves outside the app's data: left as it is", base.display());
    }
    // `remove_dir_all` does not follow symlinks inside the folder.
    std::fs::remove_dir_all(&real).with_context(|| format!("remove {}", real.display()))?;
    Ok(true)
}

#[cfg(test)]
#[path = "feature_docs_tests.rs"]
mod run_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn legacy() -> DocsLocation {
        DocsLocation::legacy("f")
    }

    #[test]
    fn docs_are_read_only_from_the_feature_folder() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join(".orchestrator/features/f");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("spec.md"), "# S").unwrap();
        std::fs::write(dir.join("api-contract.md"), "# A").unwrap();
        assert_eq!(feature_docs(&legacy(), d.path()), vec!["spec.md", "api-contract.md"]);
        assert_eq!(read_feature_doc(&legacy(), d.path(), "spec.md").unwrap(), "# S");
        for bad in ["../x.md", "a/b.md", "spec.txt"] {
            assert!(read_feature_doc(&legacy(), d.path(), bad).is_err());
        }
    }

    #[test]
    fn assets_are_images_confined_to_the_feature_folder() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join(".orchestrator/features/f");
        std::fs::create_dir_all(dir.join("design")).unwrap();
        std::fs::write(dir.join("design/home.png"), b"\x89PNG").unwrap();
        std::fs::write(dir.join("flow.SVG"), "<svg/>").unwrap();
        std::fs::write(dir.join("notes.txt"), "x").unwrap();
        std::fs::write(d.path().join("secret.png"), b"s").unwrap();
        let big = vec![0u8; crate::artifacts::MAX_PREVIEW_BYTES as usize + 1];
        std::fs::write(dir.join("big.png"), big).unwrap();
        std::os::unix::fs::symlink(d.path().join("secret.png"), dir.join("link.png")).unwrap();
        std::os::unix::fs::symlink(d.path(), dir.join("up")).unwrap();
        let get = |p: &str| read_feature_asset(&legacy(), d.path(), p);
        let err = |p: &str| get(p).unwrap_err().to_string();

        for p in ["design/home.png", "./design/home.png"] {
            assert_eq!(get(p).unwrap(), json!({"mime": "image/png", "data_b64": "iVBORw==", "bytes": 4}), "{p}");
        }
        assert_eq!(get("flow.SVG").unwrap()["mime"], "image/svg+xml");
        for p in ["../../../secret.png", "design/../../x.png", "/etc/x.png", "link.png", "up/secret.png"] {
            assert_eq!(err(p), "outside the feature folder", "{p}");
        }
        assert_eq!(err("notes.txt"), "not an image");
        assert_eq!(err("design/missing.png"), "not found");
        assert_eq!(err("design"), "not an image");
        assert_eq!(err("big.png"), "too large (5.0 MB)");

        // A feature folder symlinked out of the worktree confines nothing.
        let out = d.path().join("elsewhere");
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(out.join("x.png"), b"x").unwrap();
        std::fs::write(out.join("spec.md"), "# S").unwrap();
        let wt = d.path().join("wt");
        std::fs::create_dir_all(wt.join(".orchestrator/features")).unwrap();
        std::os::unix::fs::symlink(&out, wt.join(".orchestrator/features/g")).unwrap();
        let g = DocsLocation::legacy("g");
        assert_eq!(read_feature_asset(&g, &wt, "x.png").unwrap_err().to_string(), "outside the feature folder");
        assert!(feature_docs(&g, &wt).is_empty());
        assert!(read_feature_doc(&g, &wt, "spec.md").is_err());
    }

    #[test]
    fn a_separate_designs_folder_previews_from_a_doc_link() {
        let d = tempfile::tempdir().unwrap();
        let loc = DocsLocation { docs: Loc::repo("docs/specs/co"), designs: Loc::repo("docs/design/co") };
        std::fs::create_dir_all(d.path().join("docs/specs/co")).unwrap();
        std::fs::create_dir_all(d.path().join("docs/design/co")).unwrap();
        std::fs::write(d.path().join("docs/specs/co/spec.md"), "# S\n![a](../../design/co/a.png)\n").unwrap();
        std::fs::write(d.path().join("docs/design/co/a.png"), b"\x89PNG").unwrap();
        std::fs::write(d.path().join("docs/specs/other.png"), b"x").unwrap();
        assert_eq!(feature_docs(&loc, d.path()), ["spec.md"]);
        assert_eq!(read_feature_asset(&loc, d.path(), "../../design/co/a.png").unwrap()["bytes"], 4);
        for p in ["../other.png", "../../design/x.png", "../../../x.png"] {
            assert_eq!(read_feature_asset(&loc, d.path(), p).unwrap_err().to_string(), "outside the feature folder");
        }
        assert_eq!(
            doc_lines(&loc, d.path(), &["spec.md".into()]),
            json!([{"path": "docs/specs/co/spec.md", "lines": 2}])
        );
    }

    #[test]
    fn app_folders_are_read_by_absolute_path_and_removed_with_the_feature() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path().canonicalize().unwrap();
        let base = app_base(&root, "p1", "co");
        let loc = orch_team::FeatureDocsSetting::from_form(
            &json!({"docs": {"kind": "app"}, "designs": {"kind": "with_docs"}}),
        )
        .unwrap()
        .resolve("co", &base.to_string_lossy());
        create_app_dirs(&loc).unwrap();
        std::fs::write(base.join("docs/spec.md"), "# S\n![h](design/h.png)\n").unwrap();
        std::fs::write(base.join("docs/design/h.png"), b"\x89PNG").unwrap();
        let wt = tempfile::tempdir().unwrap();
        assert_eq!(feature_docs(&loc, wt.path()), ["spec.md"]);
        assert_eq!(read_feature_asset(&loc, wt.path(), "design/h.png").unwrap()["bytes"], 4);
        assert_eq!(
            read_feature_asset(&loc, wt.path(), "../../x.png").unwrap_err().to_string(),
            "outside the feature folder"
        );
        let abs = base.join("docs/spec.md");
        assert_eq!(doc_lines(&loc, wt.path(), &["spec.md".into()])[0]["path"], json!(abs.to_string_lossy()));
        assert!(is_app_file(&loc, &base.join("docs/design/h.png")));
        assert!(!is_app_file(&loc, &root));
        assert!(!is_app_file(&loc, &base.join("docs/design")), "a folder is no file");
        std::os::unix::fs::symlink(base.join("docs/design/h.png"), base.join("docs/link.png")).unwrap();
        assert!(!is_app_file(&loc, &base.join("docs/link.png")), "a symlink, even inside, is refused");
        assert!(!is_app_file(&DocsLocation::legacy("co"), &base.join("docs/design/h.png")));

        // A symlink planted in place of the base is never followed.
        let keep = root.join("keep");
        std::fs::create_dir_all(&keep).unwrap();
        std::fs::write(keep.join("x"), "x").unwrap();
        let evil = app_base(&root, "p1", "evil");
        std::os::unix::fs::symlink(&keep, &evil).unwrap();
        assert!(remove_app_base(&root, "p1", "evil").is_err());
        assert!(keep.join("x").is_file());
        assert!(remove_app_base(&root, "p1", "../x").is_err());

        assert!(remove_app_base(&root, "p1", "co").unwrap());
        assert!(!base.exists() && root.join("projects/p1/features").is_dir());
        assert!(!remove_app_base(&root, "p1", "co").unwrap(), "nothing left to remove");
    }
}
