//! Where a feature's docs and designs are saved (change-how-specs-are-saved
//! spec §3): the project's setting ([`FeatureDocsSetting`]) and what a
//! feature resolved from it at creation ([`DocsLocation`]).

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The legacy docs folder template, the default.
pub const DEFAULT_DOCS_PATH: &str = ".orchestrator/features/{slug}";
/// Folder names that hold code run outside the agents' sandbox (git hooks and
/// config, Claude Code settings): never part of a docs or designs folder.
pub const FORBIDDEN_COMPONENTS: &[&str] = &[".git", ".claude"];
/// Longest folder template accepted.
pub const MAX_PATH_LEN: usize = 200;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocKind {
    /// `dir` is relative to a worktree, committed like code.
    #[default]
    Repo,
    /// `dir` is absolute, in the app's data: never committed.
    App,
}

/// One resolved folder.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Loc {
    pub kind: LocKind,
    /// No trailing `/`.
    pub dir: String,
}

impl Loc {
    pub fn repo(dir: impl Into<String>) -> Self {
        Loc { kind: LocKind::Repo, dir: dir.into() }
    }
    pub fn app(dir: impl Into<String>) -> Self {
        Loc { kind: LocKind::App, dir: dir.into() }
    }
    pub fn is_app(&self) -> bool {
        self.kind == LocKind::App
    }
    /// The folder for a worktree: a repo dir inside it, an app dir as is.
    pub fn dir_in(&self, worktree: &std::path::Path) -> std::path::PathBuf {
        match self.kind {
            LocKind::Repo => worktree.join(&self.dir),
            LocKind::App => std::path::PathBuf::from(&self.dir),
        }
    }
    /// The folder as agents are told it, with a trailing `/`.
    pub fn display(&self) -> String {
        format!("{}/", self.dir.trim_end_matches('/'))
    }
}

/// The folders of one feature (stored on it at creation).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocsLocation {
    pub docs: Loc,
    pub designs: Loc,
}

impl DocsLocation {
    /// A feature created before the setting: `.orchestrator/features/<slug>`
    /// with `design/` inside.
    pub fn legacy(slug: &str) -> Self {
        let docs = format!(".orchestrator/features/{slug}");
        DocsLocation { designs: Loc::repo(format!("{docs}/design")), docs: Loc::repo(docs) }
    }
    /// An empty value (a `FeatureCtx` built without one) reads as `legacy`.
    pub fn or_legacy(&self, slug: &str) -> Self {
        if self.docs.dir.is_empty() { Self::legacy(slug) } else { self.clone() }
    }
    /// Designs in the docs folder's `design/` (no separate folder to name).
    pub fn designs_with_docs(&self) -> bool {
        self.designs.kind == self.docs.kind && self.designs.dir == format!("{}/design", self.docs.dir)
    }
    /// The absolute app folders (docs first); designs in the docs folder's
    /// `design/` are covered by the docs folder.
    pub fn app_dirs(&self) -> Vec<String> {
        let docs = self.docs.is_app().then(|| self.docs.dir.clone());
        let designs = (self.designs.is_app() && !self.designs_with_docs()).then(|| self.designs.dir.clone());
        docs.into_iter().chain(designs).collect()
    }
}

impl DocsLocation {
    /// The repo folders as resolved for a feature: no forbidden component, which
    /// a `{slug}` substitution can build (`.g{slug}` with the slug `it`).
    pub fn check_resolved(&self) -> Result<(), String> {
        for l in [&self.docs, &self.designs].into_iter().filter(|l| !l.is_app()) {
            if l.dir.split('/').any(|c| FORBIDDEN_COMPONENTS.iter().any(|f| c.eq_ignore_ascii_case(f))) {
                return Err(format!(
                    "the folder {} has a {} component: it holds code that runs outside the sandbox",
                    l.dir,
                    FORBIDDEN_COMPONENTS.join(" or ")
                ));
            }
        }
        Ok(())
    }
}

/// The Docs row of the setting.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DocsChoice {
    Repo { path: String },
    App,
}

/// The Designs row of the setting.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DesignsChoice {
    WithDocs,
    Repo { path: String },
    App,
}

/// `ProjectSettings.feature_docs` (api-contract §1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeatureDocsSetting {
    pub docs: DocsChoice,
    pub designs: DesignsChoice,
}

impl Default for FeatureDocsSetting {
    fn default() -> Self {
        FeatureDocsSetting {
            docs: DocsChoice::Repo { path: DEFAULT_DOCS_PATH.into() },
            designs: DesignsChoice::WithDocs,
        }
    }
}

/// Checks a folder template and returns it without its trailing `/` (the
/// messages are the Settings copy, ux-brief "Final design").
pub fn check_path(raw: &str) -> Result<String, String> {
    const INSIDE: &str = "Must be a path inside the repository";
    let p = raw.trim().trim_end_matches('/');
    if p.is_empty() {
        return Err("Must contain {slug}".into());
    }
    if p.chars().count() > MAX_PATH_LEN {
        return Err(format!("Too long ({MAX_PATH_LEN} characters at most)"));
    }
    if p.starts_with('/') || p.starts_with('~') || p.contains(['\\', '\n', '\r', '\0']) {
        return Err(INSIDE.into());
    }
    if p.split('/').any(|c| {
        c.is_empty() || c == ".." || c == "." || FORBIDDEN_COMPONENTS.iter().any(|f| c.eq_ignore_ascii_case(f))
    }) {
        return Err(INSIDE.into());
    }
    if p.replace("{slug}", "").contains(['{', '}']) {
        return Err(INSIDE.into());
    }
    if !p.contains("{slug}") {
        return Err("Must contain {slug}".into());
    }
    Ok(p.to_string())
}

impl FeatureDocsSetting {
    /// The form's value, checked: an error names the field
    /// (`feature_docs.docs.path: must contain {slug}`).
    pub fn from_form(v: &Value) -> Result<Self, String> {
        let row = |name: &str| -> Result<(String, Option<String>), String> {
            let r = v.get(name).ok_or_else(|| format!("feature_docs.{name}: is required"))?;
            let kind = r
                .get("kind")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("feature_docs.{name}.kind: is required"))?;
            let path = match r.get("path") {
                None | Some(Value::Null) => None,
                Some(Value::String(s)) => Some(s.clone()),
                Some(_) => return Err(format!("feature_docs.{name}.path: must be a string")),
            };
            Ok((kind.to_string(), path))
        };
        let repo_path = |name: &str, path: Option<String>| -> Result<String, String> {
            check_path(path.as_deref().unwrap_or("")).map_err(|m| format!("feature_docs.{name}.path: {m}"))
        };
        let docs = match row("docs")? {
            (k, p) if k == "repo" => DocsChoice::Repo { path: repo_path("docs", p)? },
            (k, _) if k == "app" => DocsChoice::App,
            (k, _) => return Err(format!("feature_docs.docs.kind: unknown {k:?} (repo or app)")),
        };
        let designs = match row("designs")? {
            (k, _) if k == "with_docs" => DesignsChoice::WithDocs,
            (k, p) if k == "repo" => DesignsChoice::Repo { path: repo_path("designs", p)? },
            (k, _) if k == "app" => DesignsChoice::App,
            (k, _) => return Err(format!("feature_docs.designs.kind: unknown {k:?} (with_docs, repo or app)")),
        };
        Ok(FeatureDocsSetting { docs, designs })
    }

    /// The folders a feature `slug` gets: repo templates filled in, app ones
    /// under `app_base` (`<app data>/projects/<id>/features/<slug>`).
    pub fn resolve(&self, slug: &str, app_base: &str) -> DocsLocation {
        let fill = |p: &str| p.trim_end_matches('/').replace("{slug}", slug);
        let base = app_base.trim_end_matches('/');
        let docs = match &self.docs {
            DocsChoice::Repo { path } => Loc::repo(fill(path)),
            DocsChoice::App => Loc::app(format!("{base}/docs")),
        };
        let designs = match &self.designs {
            // Always `design/` in the docs folder, so a doc's `design/x.png` resolves.
            DesignsChoice::WithDocs => Loc { kind: docs.kind, dir: format!("{}/design", docs.dir) },
            DesignsChoice::Repo { path } => Loc::repo(fill(path)),
            DesignsChoice::App => Loc::app(format!("{base}/design")),
        };
        DocsLocation { docs, designs }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn forbidden_folders_are_refused_in_the_template_and_once_resolved() {
        for p in [".claude/{slug}", "docs/.Claude/{slug}", ".GIT/{slug}"] {
            assert!(check_path(p).is_err(), "{p}");
        }
        let s = FeatureDocsSetting {
            docs: DocsChoice::Repo { path: ".g{slug}/docs".into() },
            designs: DesignsChoice::WithDocs,
        };
        assert!(s.resolve("co", "/a").check_resolved().is_ok());
        let e = s.resolve("it", "/a").check_resolved().unwrap_err();
        assert!(e.contains(".git"), "{e}");
        let s = FeatureDocsSetting { docs: DocsChoice::App, designs: DesignsChoice::Repo { path: ".{slug}/d".into() } };
        assert!(s.resolve("claude", "/a").check_resolved().is_err());
        assert!(FeatureDocsSetting::default().resolve("x", "/a").check_resolved().is_ok());
    }

    #[test]
    fn default_resolves_to_the_legacy_folders() {
        let d = FeatureDocsSetting::default().resolve("checkout", "/x");
        assert_eq!(d, DocsLocation::legacy("checkout"));
        assert!(d.designs_with_docs() && d.app_dirs().is_empty());
        assert_eq!(d.docs.display(), ".orchestrator/features/checkout/");
        assert_eq!(DocsLocation::default().or_legacy("a"), DocsLocation::legacy("a"));
    }

    #[test]
    fn resolves_each_choice() {
        let s = |v: Value| FeatureDocsSetting::from_form(&v).unwrap();
        let d = s(json!({"docs": {"kind": "repo", "path": "docs/specs/{slug}/"}, "designs": {"kind": "with_docs"}}))
            .resolve("co", "/app/p/features/co");
        assert_eq!(d.docs, Loc::repo("docs/specs/co"));
        assert_eq!(d.designs, Loc::repo("docs/specs/co/design"));
        let d = s(json!({"docs": {"kind": "app"}, "designs": {"kind": "with_docs"}})).resolve("co", "/app/f/co/");
        assert_eq!(d.docs, Loc::app("/app/f/co/docs"));
        assert_eq!(d.designs, Loc::app("/app/f/co/docs/design"));
        assert!(d.designs_with_docs());
        assert_eq!(d.app_dirs(), ["/app/f/co/docs"], "the designs folder is inside the docs one");
        let d = s(json!({"docs": {"kind": "app"}, "designs": {"kind": "app"}})).resolve("co", "/a");
        assert_eq!(d.app_dirs(), ["/a/docs", "/a/design"]);
        let d =
            s(json!({"docs": {"kind": "repo", "path": "d/{slug}"}, "designs": {"kind": "app"}})).resolve("co", "/a");
        assert_eq!((d.docs, d.designs), (Loc::repo("d/co"), Loc::app("/a/design")));
        let d = s(json!({"docs": {"kind": "app"}, "designs": {"kind": "repo", "path": "docs/design/{slug}"}}))
            .resolve("co", "/a");
        assert_eq!((d.docs, d.designs), (Loc::app("/a/docs"), Loc::repo("docs/design/co")));
    }

    #[test]
    fn invalid_folders_are_refused_with_the_field() {
        let err = |docs: &str| {
            FeatureDocsSetting::from_form(
                &json!({"docs": {"kind": "repo", "path": docs}, "designs": {"kind": "with_docs"}}),
            )
            .unwrap_err()
        };
        let inside = "feature_docs.docs.path: Must be a path inside the repository";
        for p in [
            "/abs/{slug}",
            "../x/{slug}",
            ".git/{slug}",
            "a/.GIT/{slug}",
            "./{slug}",
            "a\\{slug}",
            "a//{slug}",
            "~/{slug}",
            "{slug}/{name}",
        ] {
            assert_eq!(err(p), inside, "{p}");
        }
        assert_eq!(err("docs/specs"), "feature_docs.docs.path: Must contain {slug}");
        assert_eq!(err(""), "feature_docs.docs.path: Must contain {slug}");
        assert_eq!(
            err(&format!("{}{{slug}}", "a".repeat(200))),
            "feature_docs.docs.path: Too long (200 characters at most)"
        );
        let e = FeatureDocsSetting::from_form(
            &json!({"docs": {"kind": "app"}, "designs": {"kind": "repo", "path": "/x/{slug}"}}),
        )
        .unwrap_err();
        assert_eq!(e, "feature_docs.designs.path: Must be a path inside the repository");
        assert!(FeatureDocsSetting::from_form(&json!({"docs": {"kind": "x"}, "designs": {"kind": "app"}})).is_err());
        // Valid: a bare `{slug}`, a trailing `/` (dropped), designs = docs.
        let ok = FeatureDocsSetting::from_form(
            &json!({"docs": {"kind": "repo", "path": "{slug}/"}, "designs": {"kind": "repo", "path": "{slug}"}}),
        )
        .unwrap();
        assert_eq!(ok.docs, DocsChoice::Repo { path: "{slug}".into() });
        assert_eq!(ok.resolve("co", "/a").designs, Loc::repo("co"));
    }

    #[test]
    fn setting_serializes_as_the_contract() {
        assert_eq!(
            serde_json::to_value(FeatureDocsSetting::default()).unwrap(),
            json!({"docs": {"kind": "repo", "path": ".orchestrator/features/{slug}"}, "designs": {"kind": "with_docs"}})
        );
        assert_eq!(
            serde_json::to_value(DocsLocation::legacy("a")).unwrap(),
            json!({"docs": {"kind": "repo", "dir": ".orchestrator/features/a"},
                   "designs": {"kind": "repo", "dir": ".orchestrator/features/a/design"}})
        );
    }
}
