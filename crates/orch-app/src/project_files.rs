//! FR-112, Project settings › Files: the project's external files and
//! folders (api-contract). The checks live in
//! [`agent_runtime::project_files`]; this keeps the list, audits each change
//! and answers the UI.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use agent_runtime::project_files::{self as pf, Bounds};
use anyhow::{Result, anyhow, bail};
use orch_store::{FileAccess, Project, ProjectFile};
use serde_json::{Value, json};

use crate::LockOk;
use crate::core::Core;

/// Serializes the read-modify-write of a project's settings (the list, and
/// the General form that must keep it).
pub(crate) static SETTINGS_EDIT: Mutex<()> = Mutex::new(());

/// A stored entry as the UI gets it: `exists` checked now.
fn view(f: &ProjectFile) -> Value {
    let mut v = serde_json::to_value(f).unwrap_or_default();
    v["exists"] = json!(f.path.exists());
    v
}

fn views(files: &[ProjectFile]) -> Vec<Value> {
    files.iter().map(view).collect()
}

fn not_listed(path: &str) -> anyhow::Error {
    anyhow!("{path}: not in the project files")
}

impl Core {
    fn files_project(&self, project_id: &str) -> Result<Project> {
        self.runtime()?.db.project(project_id)?.ok_or_else(|| anyhow!("no project {project_id}"))
    }

    fn bounds(&self, p: &Project, home: &Path) -> Result<Bounds> {
        let rt = self.runtime()?;
        let all = rt.db.projects(true)?;
        Ok(Bounds::of(p, &all, home, &rt.cfg.paths.root))
    }

    /// Saves `files` as the project's list, the rest of its settings as stored.
    fn save_files(&self, p: &Project, files: Vec<ProjectFile>) -> Result<()> {
        let mut s = p.typed_settings();
        s.external_files = files;
        self.runtime()?.db.set_project_settings(&p.id, &s, &p.backend(), p.track_orchestrator_dir)
    }

    fn audit_file(&self, project_id: &str, action: &str, detail: Value) {
        if let Ok(rt) = self.runtime() {
            let _ = rt.db.audit("user", None, action, Some(project_id), "ok", detail);
        }
    }

    /// `project_files(project) -> {files, live_agents}`.
    pub fn project_files(&self, project_id: &str) -> Result<Value> {
        let p = self.files_project(project_id)?;
        let live = self.runtime()?.live_in_project(project_id);
        Ok(json!({"files": views(&p.typed_settings().external_files), "live_agents": live}))
    }

    /// `project_files_add(project, paths, access?) -> {files, added, refused}`.
    pub fn project_files_add(&self, project_id: &str, paths: &[String], access: Option<FileAccess>) -> Result<Value> {
        self.project_files_add_in(project_id, paths, access, &agent_runtime::paths::home_dir())
    }

    pub(crate) fn project_files_add_in(
        &self,
        project_id: &str,
        paths: &[String],
        access: Option<FileAccess>,
        home: &Path,
    ) -> Result<Value> {
        let _g = SETTINGS_EDIT.lock_ok();
        let p = self.files_project(project_id)?;
        let bounds = self.bounds(&p, home)?;
        let mut files = p.typed_settings().external_files;
        let access = access.unwrap_or_default();
        let (added, refused) = pf::add_batch(&mut files, paths, access, &bounds, orch_store::now_ms());
        if !added.is_empty() {
            self.save_files(&p, files.clone())?;
            for f in files.iter().filter(|f| added.contains(&f.path)) {
                self.audit_file(
                    project_id,
                    "project_file_added",
                    json!({"path": f.path, "kind": f.kind.as_str(), "access": f.access.as_str()}),
                );
            }
        }
        Ok(json!({"files": views(&files), "added": added, "refused": refused}))
    }

    /// `project_file_update(project, path, access?, note?) -> {files}`.
    /// `note: ""` clears it; `None` leaves it as is.
    pub fn project_file_update(
        &self,
        project_id: &str,
        path: &str,
        access: Option<FileAccess>,
        note: Option<&str>,
    ) -> Result<Value> {
        let _g = SETTINGS_EDIT.lock_ok();
        let p = self.files_project(project_id)?;
        let mut files = p.typed_settings().external_files;
        let i = files.iter().position(|f| f.path == Path::new(path)).ok_or_else(|| not_listed(path))?;
        let note = match note {
            Some(n) => Some(pf::normalize_note(Some(n)).map_err(|e| anyhow!(e))?),
            None => None,
        };
        let f = &files[i];
        let new_access = access.unwrap_or(f.access);
        if new_access != f.access {
            pf::check_nesting(&f.path, f.kind, new_access, &files).map_err(|m| anyhow!("{path}: {m}"))?;
        }
        let note_changed = note.as_ref().is_some_and(|n| *n != f.note);
        let access_changed = new_access != f.access;
        if access_changed || note_changed {
            let f = &mut files[i];
            f.access = new_access;
            if let Some(n) = note {
                f.note = n;
            }
            self.save_files(&p, files.clone())?;
            let mut detail = json!({"path": path, "note_changed": note_changed});
            if access_changed {
                detail["access"] = json!(new_access.as_str());
            }
            self.audit_file(project_id, "project_file_changed", detail);
        }
        Ok(json!({"files": views(&files)}))
    }

    /// `project_file_remove(project, path) -> {files}`: the entry only, never the file.
    pub fn project_file_remove(&self, project_id: &str, path: &str) -> Result<Value> {
        let _g = SETTINGS_EDIT.lock_ok();
        let p = self.files_project(project_id)?;
        let mut files = p.typed_settings().external_files;
        let n = files.len();
        files.retain(|f| f.path != Path::new(path));
        if files.len() == n {
            return Err(not_listed(path));
        }
        self.save_files(&p, files.clone())?;
        self.audit_file(project_id, "project_file_removed", json!({"path": path}));
        Ok(json!({"files": views(&files)}))
    }

    /// The listed entry at `path` in any project (what `reveal_path` may show).
    pub fn listed_project_file(&self, path: &str) -> Result<PathBuf> {
        let rt = self.runtime()?;
        let want = std::fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path));
        let listed = rt
            .db
            .projects(false)?
            .iter()
            .flat_map(|p| p.typed_settings().external_files)
            .any(|f| f.path == want || f.path == Path::new(path));
        if !listed {
            bail!("{path}: not in the project files");
        }
        Ok(want)
    }

    /// Reveal in Finder (`open -R`), for a listed project file only.
    pub fn reveal_path(&self, path: &str) -> Result<()> {
        let p = self.listed_project_file(path)?;
        let st = std::process::Command::new("open").arg("-R").arg(&p).status()?;
        if !st.success() {
            bail!("{path}: Finder could not show it");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::tests::test_core;

    struct Fx {
        _d: tempfile::TempDir,
        core: std::sync::Arc<Core>,
        home: PathBuf,
    }

    /// A core on project `repo`, a fake user home with credentials and
    /// files to attach.
    fn fx() -> Fx {
        let d = tempfile::Builder::new().prefix("orchpf").tempdir_in(orch_protocol::test_tmp()).unwrap();
        let core = test_core(d.path());
        std::fs::create_dir_all(d.path().join("user/.ssh")).unwrap();
        std::fs::create_dir_all(d.path().join("user/Brand")).unwrap();
        std::fs::create_dir_all(d.path().join("repo/src")).unwrap();
        std::fs::write(d.path().join("user/app.figcraft"), "x").unwrap();
        std::fs::write(d.path().join("user/api.yaml"), "x").unwrap();
        let home = std::fs::canonicalize(d.path().join("user")).unwrap();
        Fx { _d: d, core, home }
    }

    fn strs(v: &Value) -> Vec<String> {
        v.as_array().unwrap().iter().map(|s| s.as_str().unwrap().to_string()).collect()
    }

    fn audits(core: &Core, action: &str) -> Vec<Value> {
        let rt = core.runtime().unwrap();
        rt.db.audit_log(None, 100).unwrap().into_iter().filter(|a| a.action == action).map(|a| a.detail).collect()
    }

    /// Acceptance 1–2: a batch adds the valid paths, refuses the rest with
    /// their messages, `~` expanded.
    #[test]
    fn add_a_batch_with_refusals() {
        let f = fx();
        let repo_src = f._d.path().join("repo/src").display().to_string();
        let paths: Vec<String> =
            ["~/Brand", "~/app.figcraft", "~/api.yaml", "~/.ssh", "~", &repo_src, "~/gone"].map(String::from).into();
        let r = f.core.project_files_add_in("repo", &paths, None, &f.home).unwrap();
        assert_eq!(
            strs(&r["added"]),
            ["Brand", "app.figcraft", "api.yaml"].map(|p| f.home.join(p).display().to_string())
        );
        let refused: Vec<(String, String)> = r["refused"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| (x["path"].as_str().unwrap().into(), x["error"].as_str().unwrap().into()))
            .collect();
        assert_eq!(
            refused,
            [
                ("~/.ssh".into(), "~/.ssh: holds credentials, can't be shared with agents".into()),
                ("~".into(), "~: too broad, pick a folder inside it".into()),
                (repo_src.clone(), format!("{repo_src}: already part of the project (in its repository)")),
                ("~/gone".into(), "~/gone: no such file or folder".into()),
            ]
        );
        let files = r["files"].as_array().unwrap();
        assert_eq!(files[0]["kind"], "folder");
        assert_eq!(files[1]["kind"], "file");
        assert!(files.iter().all(|f| f["access"] == "write" && f.get("note").is_none() && f["exists"] == true));
        let v = f.core.project_files("repo").unwrap();
        assert_eq!(v["files"], r["files"]);
        assert_eq!(v["live_agents"], 0);
        assert_eq!(audits(&f.core, "project_file_added").len(), 3);
        assert_eq!(audits(&f.core, "project_file_added")[0]["access"], "write");
        // An unknown project is an error.
        assert!(f.core.project_files_add_in("nope", &paths, None, &f.home).is_err());
        f.core.shutdown();
    }

    #[test]
    fn update_access_and_note() {
        let f = fx();
        let r = f.core.project_files_add_in("repo", &["~/Brand".into(), "~/api.yaml".into()], None, &f.home).unwrap();
        let brand = strs(&r["added"])[0].clone();
        let r = f.core.project_file_update("repo", &brand, Some(FileAccess::Read), Some("  Brand guide ")).unwrap();
        assert_eq!(
            (r["files"][0]["access"].as_str(), r["files"][0]["note"].as_str()),
            (Some("read"), Some("Brand guide"))
        );
        // Omitted note: unchanged; "" clears it.
        let r = f.core.project_file_update("repo", &brand, Some(FileAccess::Write), None).unwrap();
        assert_eq!(r["files"][0]["note"], "Brand guide");
        let r = f.core.project_file_update("repo", &brand, None, Some("")).unwrap();
        assert!(r["files"][0].get("note").is_none());
        let long = "a".repeat(501);
        let e = f.core.project_file_update("repo", &brand, None, Some(&long)).unwrap_err();
        assert_eq!(e.to_string(), "note: 500 characters at most");
        let e = f.core.project_file_update("repo", "/nope", None, None).unwrap_err();
        assert_eq!(e.to_string(), "/nope: not in the project files");
        let changed = audits(&f.core, "project_file_changed");
        assert_eq!(changed.len(), 3, "{changed:?}");
        // Newest first: the note cleared, then access back to write, then read + note.
        assert_eq!(changed[0], json!({"path": brand, "note_changed": true}));
        assert_eq!(changed[1], json!({"path": brand, "access": "write", "note_changed": false}));
        assert_eq!(changed[2], json!({"path": brand, "access": "read", "note_changed": true}));
        f.core.shutdown();
    }

    /// A read folder can't hold write entries (Claude Code denies win).
    #[test]
    fn nested_access_is_refused_both_ways() {
        let f = fx();
        std::fs::create_dir_all(f.home.join("Brand/logos")).unwrap();
        let brand = f.home.join("Brand").display().to_string();
        f.core.project_files_add_in("repo", &["~/Brand/logos".into()], None, &f.home).unwrap();
        f.core.project_files_add_in("repo", &["~/Brand".into()], None, &f.home).unwrap();
        let e = f.core.project_file_update("repo", &brand, Some(FileAccess::Read), None).unwrap_err();
        assert_eq!(e.to_string(), format!("{brand}: contains files set to Read & write; set them to Read only first"));
        let logos = f.home.join("Brand/logos").display().to_string();
        f.core.project_file_update("repo", &logos, Some(FileAccess::Read), None).unwrap();
        f.core.project_file_update("repo", &brand, Some(FileAccess::Read), None).unwrap();
        let e = f.core.project_file_update("repo", &logos, Some(FileAccess::Write), None).unwrap_err();
        assert_eq!(
            e.to_string(),
            format!("{logos}: inside a read-only folder in the project files; set that folder to Read & write first")
        );
        f.core.shutdown();
    }

    /// Acceptance 6: removing leaves the file; the General form never erases the list.
    #[test]
    fn remove_keeps_the_file_and_settings_form_keeps_the_list() {
        let f = fx();
        let r =
            f.core.project_files_add_in("repo", &["~/Brand".into(), "~/app.figcraft".into()], None, &f.home).unwrap();
        let fig = strs(&r["added"])[1].clone();
        let form = |files: Value| -> crate::teams::ProjectSettingsForm {
            serde_json::from_value(json!({"settings": {"default_team": "full-stack", "external_files": files},
                "backend": {"type": "local"}, "track_orchestrator_dir": true}))
            .unwrap()
        };
        let before = f.core.project_files("repo").unwrap()["files"].clone();
        for files in [json!([]), json!([{"path": "/etc", "kind": "folder", "added_at_ms": 1}])] {
            let s = f.core.project_settings_set("repo", form(files)).unwrap();
            assert_eq!(s["settings"]["default_team"], "full-stack");
            assert_eq!(f.core.project_files("repo").unwrap()["files"], before, "the list is kept");
        }
        // A form without the key keeps it too.
        let f2: crate::teams::ProjectSettingsForm = serde_json::from_value(
            json!({"settings": {}, "backend": {"type": "local"}, "track_orchestrator_dir": true}),
        )
        .unwrap();
        f.core.project_settings_set("repo", f2).unwrap();
        assert_eq!(f.core.project_files("repo").unwrap()["files"], before);

        let r = f.core.project_file_remove("repo", &fig).unwrap();
        assert_eq!(r["files"].as_array().unwrap().len(), 1);
        assert!(Path::new(&fig).exists(), "the file on disk is untouched");
        assert_eq!(
            f.core.project_file_remove("repo", &fig).unwrap_err().to_string(),
            format!("{fig}: not in the project files")
        );
        assert_eq!(audits(&f.core, "project_file_removed"), [json!({"path": fig})]);
        // A missing entry shows as such (acceptance 7).
        std::fs::remove_dir_all(f.home.join("Brand")).unwrap();
        assert_eq!(f.core.project_files("repo").unwrap()["files"][0]["exists"], false);
        f.core.shutdown();
    }

    /// One read-modify-write under one lock: concurrent adds and General
    /// form saves never drop an entry.
    #[test]
    fn concurrent_saves_keep_every_entry() {
        let f = fx();
        let names: Vec<String> = (0..8).map(|i| format!("f{i}.md")).collect();
        for n in &names {
            std::fs::write(f.home.join(n), "x").unwrap();
        }
        std::thread::scope(|sc| {
            for n in &names {
                let (core, home) = (&f.core, &f.home);
                sc.spawn(move || core.project_files_add_in("repo", &[format!("~/{n}")], None, home).unwrap());
            }
            for _ in 0..4 {
                sc.spawn(|| {
                    let form: crate::teams::ProjectSettingsForm = serde_json::from_value(
                        json!({"settings": {}, "backend": {"type": "local"}, "track_orchestrator_dir": true}),
                    )
                    .unwrap();
                    f.core.project_settings_set("repo", form).unwrap();
                });
            }
        });
        assert_eq!(f.core.project_files("repo").unwrap()["files"].as_array().unwrap().len(), names.len());
        f.core.shutdown();
    }

    #[test]
    fn reveal_only_listed_paths() {
        let f = fx();
        f.core.project_files_add_in("repo", &["~/Brand".into()], None, &f.home).unwrap();
        let brand = f.home.join("Brand");
        assert_eq!(f.core.listed_project_file(&brand.display().to_string()).unwrap(), brand);
        let other = f.home.join("api.yaml").display().to_string();
        assert_eq!(f.core.reveal_path(&other).unwrap_err().to_string(), format!("{other}: not in the project files"));
        f.core.shutdown();
    }
}
