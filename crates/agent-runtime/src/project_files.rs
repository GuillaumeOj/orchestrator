//! FR-112: files and folders outside the repository attached to a project.
//! Validation when the user adds or changes one, and again at each spawn
//! (spec › Validation). The path checks are pure; only [`expand`]'s callers
//! touch the disk (canonicalize).

use std::path::{Path, PathBuf};

use orch_store::{FileAccess, FileKind, Project, ProjectFile};

use crate::compile::CREDENTIAL_STORES;

/// At most this many entries per project.
pub const MAX_FILES: usize = 100;
/// A note's length limit, in characters.
pub const MAX_NOTE: usize = 500;

/// What a project file may not touch.
#[derive(Clone, Debug, Default)]
pub struct Bounds {
    pub home: PathBuf,
    pub app_root: PathBuf,
    pub main_checkout: PathBuf,
    pub worktree_root: PathBuf,
    /// The main checkouts of the other projects.
    pub other_repos: Vec<PathBuf>,
    /// The worktree roots of the other projects (agents' worktrees).
    pub other_worktree_roots: Vec<PathBuf>,
}

impl Bounds {
    /// The bounds of `project` among `all` (it may be in `all`). Paths are
    /// canonicalized when they exist so they compare with canonical entries.
    pub fn of(project: &Project, all: &[Project], home: &Path, app_root: &Path) -> Bounds {
        Bounds {
            home: canon_or(home),
            app_root: canon_or(app_root),
            main_checkout: canon_or(&project.path),
            worktree_root: canon_or(&project.worktree_root),
            other_repos: all.iter().filter(|p| p.id != project.id).map(|p| canon_or(&p.path)).collect(),
            other_worktree_roots: all
                .iter()
                .filter(|p| p.id != project.id)
                .map(|p| canon_or(&p.worktree_root))
                .collect(),
        }
    }
}

fn canon_or(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// `~` and `~/…` expanded; anything still relative is refused.
pub fn expand(input: &str, home: &Path) -> Result<PathBuf, String> {
    let s = input.trim();
    let p = if s == "~" {
        home.to_path_buf()
    } else if let Some(rest) = s.strip_prefix("~/") {
        home.join(rest)
    } else {
        PathBuf::from(s)
    };
    if p.is_absolute() { Ok(p) } else { Err(format!("{s}: use an absolute path or ~/…")) }
}

/// Whether `canon` (canonical, absolute) may be shared with the project's
/// agents at all. The message follows `{p}: `.
pub fn check(canon: &Path, b: &Bounds) -> Result<(), &'static str> {
    let inside = |outer: &Path| canon.starts_with(outer);
    let contains = |inner: &Path| inner.starts_with(canon);
    if inside(&b.main_checkout) || inside(&b.worktree_root) {
        return Err("already part of the project (in its repository)");
    }
    if b.other_worktree_roots.iter().any(|r| inside(r)) {
        return Err("inside agent worktrees, pick the file in its project instead");
    }
    if canon.parent().is_none() || contains(&b.home) {
        return Err("too broad, pick a folder inside it");
    }
    if inside(&b.app_root) || contains(&b.app_root) {
        return Err("the app's own data can't be shared with agents");
    }
    if CREDENTIAL_STORES.iter().map(|(p, _)| b.home.join(p)).any(|c| inside(&c) || contains(&c)) {
        return Err("holds credentials, can't be shared with agents");
    }
    if contains(&b.main_checkout) || b.other_repos.iter().any(|r| contains(r)) {
        return Err("contains a project repository, pick a folder inside it");
    }
    Ok(())
}

/// Claude Code deny rules beat allow rules, so a `write` entry can't live
/// under a `read` folder (its Edit deny would win), and a folder can't turn
/// `read` over `write` entries. An entry at `path` itself is not compared
/// (it is the one being changed).
pub fn check_nesting(
    path: &Path,
    kind: FileKind,
    access: FileAccess,
    files: &[ProjectFile],
) -> Result<(), &'static str> {
    let others = files.iter().filter(|f| f.path != path);
    match access {
        FileAccess::Write => {
            if others
                .clone()
                .any(|f| f.kind == FileKind::Folder && f.access == FileAccess::Read && path.starts_with(&f.path))
            {
                return Err("inside a read-only folder in the project files; set that folder to Read & write first");
            }
        }
        FileAccess::Read => {
            if kind == FileKind::Folder
                && others.clone().any(|f| f.access == FileAccess::Write && f.path.starts_with(path))
            {
                return Err("contains files set to Read & write; set them to Read only first");
            }
        }
    }
    Ok(())
}

/// A typed, picked or dropped path, made canonical and checked against the
/// bounds and the entries already listed. Errors are the full message.
pub fn resolve(input: &str, b: &Bounds, files: &[ProjectFile]) -> Result<(PathBuf, FileKind), String> {
    let shown = input.trim();
    let p = expand(input, &b.home)?;
    let canon = std::fs::canonicalize(&p).map_err(|_| format!("{shown}: no such file or folder"))?;
    if files.iter().any(|f| f.path == canon) {
        return Err(format!("{shown}: already in the project files"));
    }
    check(&canon, b).map_err(|m| format!("{shown}: {m}"))?;
    let kind = if canon.is_dir() { FileKind::Folder } else { FileKind::File };
    Ok((canon, kind))
}

/// A refused input with its message (api-contract `refused`).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Refused {
    pub path: String,
    pub error: String,
}

/// Adds the valid `inputs` to `files` (all-or-some); returns the canonical
/// paths added and the refused inputs. Duplicates inside one call count once.
pub fn add_batch(
    files: &mut Vec<ProjectFile>,
    inputs: &[String],
    access: FileAccess,
    b: &Bounds,
    now_ms: u64,
) -> (Vec<PathBuf>, Vec<Refused>) {
    let (mut added, mut refused) = (Vec::new(), Vec::new());
    for input in inputs.iter().filter(|i| !i.trim().is_empty()) {
        let shown = input.trim();
        // Already added by this very call: counts once.
        let canon = expand(input, &b.home).ok().and_then(|p| std::fs::canonicalize(p).ok());
        if canon.is_some_and(|c| added.contains(&c)) {
            continue;
        }
        let checked = resolve(input, b, files).and_then(|(path, kind)| {
            check_nesting(&path, kind, access, files).map_err(|m| format!("{shown}: {m}"))?;
            if files.len() >= MAX_FILES {
                return Err(format!("{shown}: {MAX_FILES} files at most per project"));
            }
            Ok((path, kind))
        });
        let (path, kind) = match checked {
            Ok(r) => r,
            Err(error) => {
                refused.push(Refused { path: input.clone(), error });
                continue;
            }
        };
        files.push(ProjectFile { path: path.clone(), kind, access, note: None, added_at_ms: now_ms });
        added.push(path);
    }
    (added, refused)
}

/// A note as stored: trimmed, empty = none, at most [`MAX_NOTE`] characters.
pub fn normalize_note(note: Option<&str>) -> Result<Option<String>, String> {
    let n = note.map(str::trim).filter(|n| !n.is_empty());
    match n {
        Some(n) if n.chars().count() > MAX_NOTE => Err(format!("note: {MAX_NOTE} characters at most")),
        n => Ok(n.map(str::to_string)),
    }
}

/// A project file as an agent gets it at spawn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpawnFile {
    pub path: PathBuf,
    pub kind: FileKind,
    pub access: FileAccess,
    pub note: Option<String>,
    /// Gone at spawn: listed in the prompt only, no rules.
    pub missing: bool,
}

/// The entries re-checked at spawn: re-canonicalized (a symlink may have
/// moved), missing ones kept for the prompt, and the ones that now fail the
/// bounds dropped and returned with their message for the log.
pub fn at_spawn(files: &[ProjectFile], b: &Bounds) -> (Vec<SpawnFile>, Vec<String>) {
    let (mut out, mut skipped) = (Vec::new(), Vec::new());
    for f in files {
        let spawn = |path: PathBuf, missing: bool| SpawnFile {
            path,
            kind: f.kind,
            access: f.access,
            note: f.note.clone(),
            missing,
        };
        match std::fs::canonicalize(&f.path) {
            Err(_) => out.push(spawn(f.path.clone(), true)),
            Ok(c) => match check(&c, b) {
                Ok(()) => out.push(spawn(c, false)),
                Err(m) => skipped.push(format!("{}: {m}", f.path.display())),
            },
        }
    }
    (out, skipped)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fx {
        _d: tempfile::TempDir,
        b: Bounds,
        root: PathBuf,
    }

    /// A fake disk: home, its credential stores, the app data, a project
    /// with its worktree root, another project, and some free files.
    fn fx() -> Fx {
        let d = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(d.path()).unwrap();
        let home = root.join("home");
        for p in ["home/.ssh", "home/.aws", "home/Library/Keychains", "home/Library/Application Support/Orchestrator"] {
            std::fs::create_dir_all(root.join(p)).unwrap();
        }
        for p in [
            "home/dev/shop/src",
            "home/dev/shop.worktrees/solo/x",
            "home/dev/other/lib",
            "home/dev/other.worktrees/solo/y",
            "home/Brand/logos",
        ] {
            std::fs::create_dir_all(root.join(p)).unwrap();
        }
        std::fs::write(home.join("Brand/guide.md"), "x").unwrap();
        std::fs::write(home.join("app.figcraft"), "x").unwrap();
        let b = Bounds {
            app_root: home.join("Library/Application Support/Orchestrator"),
            main_checkout: home.join("dev/shop"),
            worktree_root: home.join("dev/shop.worktrees"),
            other_repos: vec![home.join("dev/other")],
            other_worktree_roots: vec![home.join("dev/other.worktrees")],
            home,
        };
        Fx { _d: d, b, root }
    }

    fn err(f: &Fx, input: &str) -> String {
        resolve(input, &f.b, &[]).unwrap_err()
    }

    #[test]
    fn tilde_is_expanded_and_relative_refused() {
        let f = fx();
        assert_eq!(resolve("~/Brand", &f.b, &[]).unwrap(), (f.b.home.join("Brand"), FileKind::Folder));
        assert_eq!(resolve(" ~/app.figcraft\n", &f.b, &[]).unwrap(), (f.b.home.join("app.figcraft"), FileKind::File));
        assert_eq!(err(&f, "Brand"), "Brand: use an absolute path or ~/…");
        assert_eq!(err(&f, "~bob/x"), "~bob/x: use an absolute path or ~/…");
    }

    #[test]
    fn each_refusal_has_its_message() {
        let f = fx();
        let h = f.b.home.display().to_string();
        assert_eq!(err(&f, "~/nope"), "~/nope: no such file or folder");
        assert_eq!(
            err(&f, "~/dev/other.worktrees/solo/y"),
            "~/dev/other.worktrees/solo/y: inside agent worktrees, pick the file in its project instead"
        );
        assert_eq!(
            err(&f, "~/dev/other.worktrees"),
            "~/dev/other.worktrees: inside agent worktrees, pick the file in its project instead"
        );
        // Inside another project's main checkout: allowed.
        assert_eq!(resolve("~/dev/other/lib", &f.b, &[]).unwrap().1, FileKind::Folder);
        assert_eq!(err(&f, "~/dev/shop/src"), "~/dev/shop/src: already part of the project (in its repository)");
        assert_eq!(err(&f, "~/dev/shop"), "~/dev/shop: already part of the project (in its repository)");
        assert_eq!(
            err(&f, "~/dev/shop.worktrees/solo/x"),
            "~/dev/shop.worktrees/solo/x: already part of the project (in its repository)"
        );
        assert_eq!(err(&f, "/"), "/: too broad, pick a folder inside it");
        assert_eq!(err(&f, "~"), "~: too broad, pick a folder inside it");
        assert_eq!(err(&f, &h), format!("{h}: too broad, pick a folder inside it"));
        let up = f.root.display().to_string();
        assert_eq!(err(&f, &up), format!("{up}: too broad, pick a folder inside it"));
        assert_eq!(
            err(&f, "~/Library/Application Support/Orchestrator"),
            "~/Library/Application Support/Orchestrator: the app's own data can't be shared with agents"
        );
        assert_eq!(
            err(&f, "~/Library/Application Support"),
            "~/Library/Application Support: the app's own data can't be shared with agents"
        );
        assert_eq!(err(&f, "~/.ssh"), "~/.ssh: holds credentials, can't be shared with agents");
        assert_eq!(
            err(&f, "~/Library/Keychains"),
            "~/Library/Keychains: holds credentials, can't be shared with agents"
        );
        assert_eq!(err(&f, "~/Library"), "~/Library: the app's own data can't be shared with agents");
        assert_eq!(err(&f, "~/dev"), "~/dev: contains a project repository, pick a folder inside it");
    }

    #[test]
    fn credential_store_containing_folder_is_refused() {
        let f = fx();
        // Bounds without the app inside ~/Library, so the credential rule speaks.
        let b = Bounds { app_root: f.root.join("elsewhere"), ..f.b.clone() };
        let e = resolve("~/Library", &b, &[]).unwrap_err();
        assert_eq!(e, "~/Library: holds credentials, can't be shared with agents");
    }

    #[test]
    fn duplicates_and_symlinks_resolve_to_the_same_entry() {
        let f = fx();
        let link = f.b.home.join("brand-link");
        std::os::unix::fs::symlink(f.b.home.join("Brand"), &link).unwrap();
        let mut files = vec![];
        let inputs = vec!["~/Brand".to_string(), "~/brand-link".into(), "~/Brand".into()];
        let (added, refused) = add_batch(&mut files, &inputs, FileAccess::Write, &f.b, 7);
        assert_eq!(added, [f.b.home.join("Brand")]);
        assert!(refused.is_empty(), "duplicates inside one call count once: {refused:?}");
        assert_eq!(files[0].added_at_ms, 7);
        let (added, refused) = add_batch(&mut files, &["~/brand-link".into()], FileAccess::Write, &f.b, 8);
        assert!(added.is_empty());
        assert_eq!(
            refused,
            [Refused { path: "~/brand-link".into(), error: "~/brand-link: already in the project files".into() }]
        );
    }

    #[test]
    fn a_batch_adds_the_valid_and_refuses_the_rest() {
        let f = fx();
        let mut files = vec![];
        let inputs: Vec<String> =
            ["~/Brand", "~/.ssh", "~", "~/dev/shop/src", "~/nope", "~/app.figcraft", "~/Brand/guide.md"]
                .map(String::from)
                .into();
        let (added, refused) = add_batch(&mut files, &inputs, FileAccess::Write, &f.b, 1);
        assert_eq!(added.len(), 3, "a nested file is accepted");
        assert_eq!(
            refused.iter().map(|r| r.path.as_str()).collect::<Vec<_>>(),
            ["~/.ssh", "~", "~/dev/shop/src", "~/nope"]
        );
        assert_eq!(
            files.iter().map(|f| f.kind).collect::<Vec<_>>(),
            [FileKind::Folder, FileKind::File, FileKind::File]
        );
    }

    #[test]
    fn at_most_a_hundred() {
        let f = fx();
        let mut files: Vec<ProjectFile> = (0..MAX_FILES)
            .map(|i| ProjectFile {
                path: format!("/x/{i}").into(),
                kind: FileKind::File,
                access: FileAccess::Write,
                note: None,
                added_at_ms: 0,
            })
            .collect();
        let (added, refused) = add_batch(&mut files, &["~/Brand".into()], FileAccess::Write, &f.b, 1);
        assert!(added.is_empty());
        assert_eq!(refused[0].error, "~/Brand: 100 files at most per project");
    }

    #[test]
    fn nesting_rules() {
        let f = fx();
        let mut files = vec![];
        add_batch(&mut files, &["~/Brand".into()], FileAccess::Read, &f.b, 1);
        // Write inside a read folder: refused; read inside it: fine.
        let (_, refused) = add_batch(&mut files, &["~/Brand/logos".into()], FileAccess::Write, &f.b, 1);
        assert_eq!(
            refused[0].error,
            "~/Brand/logos: inside a read-only folder in the project files; set that folder to Read & write first"
        );
        let (added, _) = add_batch(&mut files, &["~/Brand/logos".into()], FileAccess::Read, &f.b, 1);
        assert_eq!(added.len(), 1);
        // A read folder over write entries.
        let mut files = vec![];
        add_batch(&mut files, &["~/Brand/guide.md".into()], FileAccess::Write, &f.b, 1);
        let brand = f.b.home.join("Brand");
        assert_eq!(
            check_nesting(&brand, FileKind::Folder, FileAccess::Read, &files),
            Err("contains files set to Read & write; set them to Read only first")
        );
        let (_, refused) = add_batch(&mut files, &["~/Brand".into()], FileAccess::Read, &f.b, 1);
        assert_eq!(refused[0].error, "~/Brand: contains files set to Read & write; set them to Read only first");
        // A read entry inside a write folder is fine (it narrows its subtree).
        let mut files = vec![];
        add_batch(&mut files, &["~/Brand".into()], FileAccess::Write, &f.b, 1);
        let (added, _) = add_batch(&mut files, &["~/Brand/guide.md".into()], FileAccess::Read, &f.b, 1);
        assert_eq!(added.len(), 1);
        // The entry being changed is not compared with itself.
        assert_eq!(check_nesting(&brand, FileKind::Folder, FileAccess::Read, &files[..1]), Ok(()));
    }

    #[test]
    fn notes() {
        assert_eq!(normalize_note(Some("  Brand guide ")), Ok(Some("Brand guide".into())));
        assert_eq!(normalize_note(Some("   ")), Ok(None));
        assert_eq!(normalize_note(None), Ok(None));
        assert_eq!(normalize_note(Some(&"é".repeat(500))).unwrap().unwrap().chars().count(), 500);
        assert_eq!(normalize_note(Some(&"a".repeat(501))), Err("note: 500 characters at most".into()));
    }

    #[test]
    fn spawn_keeps_missing_and_drops_what_now_fails() {
        let f = fx();
        let entry = |p: PathBuf| ProjectFile {
            path: p,
            kind: FileKind::Folder,
            access: FileAccess::Read,
            note: Some("n".into()),
            added_at_ms: 0,
        };
        let files = vec![entry(f.b.home.join("Brand")), entry(f.b.home.join("gone")), entry(f.b.home.join(".ssh"))];
        let (out, skipped) = at_spawn(&files, &f.b);
        assert_eq!(out.len(), 2);
        assert!(!out[0].missing && out[1].missing);
        assert_eq!(out[1].note.as_deref(), Some("n"));
        assert_eq!(
            skipped,
            [format!("{}: holds credentials, can't be shared with agents", f.b.home.join(".ssh").display())]
        );
    }

    /// Acceptance 3 and 7 through `Runtime::spawn`: the agent's files carry
    /// the rules, the command line `--add-dir`, the prompt the section; a
    /// missing entry doesn't stop the spawn.
    #[test]
    fn spawn_gives_the_agent_its_project_files() {
        let sim = crate::chaos::Sim::new(|_| {});
        let root = crate::paths::real(sim._d.path());
        let wt = root.join("wt");
        std::fs::create_dir_all(&wt).unwrap();
        for args in [
            vec!["init", "-q", "-b", "main"],
            vec!["-c", "user.name=T", "-c", "user.email=t@x", "commit", "-q", "--allow-empty", "-m", "i"],
        ] {
            assert!(std::process::Command::new("git").arg("-C").arg(&wt).args(args).status().unwrap().success());
        }
        let ext = root.join("Brand");
        std::fs::create_dir_all(&ext).unwrap();
        let p = orch_git::project::register(&sim.rt.db, &wt).unwrap();
        let entry = |path: PathBuf, note: Option<&str>| ProjectFile {
            path,
            kind: FileKind::Folder,
            access: FileAccess::Write,
            note: note.map(String::from),
            added_at_ms: 1,
        };
        let settings = orch_store::ProjectSettings {
            external_files: vec![entry(ext.clone(), Some("Brand guide")), entry(root.join("gone"), None)],
            ..Default::default()
        };
        sim.rt.db.set_project_settings(&p.id, &settings, &orch_store::TaskBackendConfig::Local, true).unwrap();
        let mut a = crate::runtime::tests::test_agent(&root);
        (a.repo, a.worktree) = (wt.clone(), wt);
        sim.rt.store.write(|s| s.agents.insert("a1".into(), a)).unwrap();

        sim.rt.spawn("a1", Some(crate::compile::SessionMode::New("s".into()))).unwrap();
        let dir = sim.rt.cfg.paths.agent_dir("a1");
        let s: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("settings.json")).unwrap()).unwrap();
        let rule = format!("Edit(/{}/**)", ext.display());
        assert!(s["permissions"]["allow"].as_array().unwrap().iter().any(|r| r == &rule), "{rule}");
        assert!(
            s["sandbox"]["filesystem"]["allowWrite"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p == &ext.display().to_string())
        );
        assert!(!s.to_string().contains("/gone"));
        sim.rt.db.flush().unwrap();
        let ev = sim.rt.db.events(0, Some("a1"), Some("spawned"), 1).unwrap();
        let mut detail = ev[0].detail.clone();
        if let Some(blob) = detail["blob"].as_str() {
            detail = serde_json::from_str(&std::fs::read_to_string(blob).unwrap()).unwrap();
        }
        let args: Vec<String> = serde_json::from_value(detail["args"].clone()).unwrap();
        assert!(args.windows(2).any(|w| w[0] == "--add-dir" && w[1] == ext.display().to_string()));
        let prompt = &args[args.iter().position(|a| a == "--append-system-prompt").unwrap() + 1];
        assert!(prompt.contains(&format!("- `{}` (folder, read & write): Brand guide", ext.display())), "{prompt}");
        assert!(prompt.contains("(folder, missing)"));
        assert_eq!(sim.rt.live_in_project(&p.id), 1);
        assert_eq!(sim.rt.live_in_project("other"), 0);
    }
}
