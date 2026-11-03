//! PR builds (spec §4.2, api-contract §1, §4): which open pull requests have a `pr-<n>`
//! prerelease built by CI, and the release to install. Parsing and joining are pure; the two
//! entry points run `gh`. Downloading, verifying and staging go through `update.rs`.

use serde::Deserialize;
use serde_json::{Value, json};

use crate::update::{GhError, RELEASE_FIELDS, REPO, VIEW_TIMEOUT, gh, parse_version};

/// A `pr-<n>` release with both its assets (api-contract §1).
#[derive(Debug, Clone, PartialEq)]
pub struct PrRelease {
    pub tag: String,
    pub version: String,
    /// The head sha the build was made from (the release's target).
    pub sha: String,
    /// The PR title (line 1 of the release body).
    pub title: String,
    pub published_at: Option<i64>,
    pub tarball: String,
    pub tarball_size: u64,
    pub sig: String,
}

/// One release as `gh release view --json …` (camelCase) or `gh api …/releases` (snake_case)
/// prints it.
#[derive(Deserialize)]
struct GhRelease {
    #[serde(alias = "tag_name", rename = "tagName")]
    tag: String,
    #[serde(alias = "prerelease", rename = "isPrerelease", default)]
    prerelease: bool,
    #[serde(alias = "target_commitish", rename = "targetCommitish", default)]
    target: String,
    #[serde(alias = "published_at", rename = "publishedAt", default)]
    published_at: Option<String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    #[serde(default)]
    size: u64,
}

fn timestamp(s: Option<&str>) -> Option<i64> {
    s.and_then(|p| p.parse::<jiff::Timestamp>().ok()).map(|t| t.as_second())
}

/// The PR number of a `pr-<n>` tag.
fn pr_number(tag: &str) -> Option<u64> {
    let n = tag.strip_prefix("pr-")?;
    (!n.starts_with('0') && n.bytes().all(|b| b.is_ascii_digit())).then(|| n.parse().ok())?
}

/// `Some` for a prerelease tagged `pr-<n>` that has its tarball and signature. The version is
/// the tarball's, never the tag's.
fn pr_release(r: GhRelease) -> Option<(u64, PrRelease)> {
    let number = pr_number(&r.tag)?;
    if !r.prerelease {
        return None;
    }
    let (version, tar) = r.assets.iter().find_map(|a| {
        let v = a.name.strip_prefix("Orchestrator_")?.strip_suffix("_aarch64.app.tar.gz")?;
        parse_version(v).is_some().then(|| (v.to_string(), a))
    })?;
    let sig = format!("{}.sig", tar.name);
    r.assets.iter().find(|a| a.name == sig)?;
    let published_at = timestamp(r.published_at.as_deref());
    let title = r.body.as_deref().and_then(|b| b.lines().next()).unwrap_or("").trim().to_string();
    Some((
        number,
        PrRelease {
            tag: r.tag.clone(),
            version,
            sha: r.target.clone(),
            title,
            published_at,
            tarball: tar.name.clone(),
            tarball_size: tar.size,
            sig,
        },
    ))
}

/// `gh release view pr-<n> --json …` → the release, if it is a complete PR build.
fn parse_pr_release(json: &str, number: u64) -> Option<PrRelease> {
    let r: GhRelease = serde_json::from_str(json).ok()?;
    pr_release(r).filter(|(n, _)| *n == number).map(|(_, rel)| rel)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhPr {
    number: u64,
    title: String,
    head_ref_name: String,
    head_ref_oid: String,
    #[serde(default)]
    author: Option<GhAuthor>,
    #[serde(default)]
    is_draft: bool,
    #[serde(default)]
    updated_at: Option<String>,
    url: String,
}

#[derive(Deserialize)]
struct GhAuthor {
    login: String,
}

/// The open PRs joined with their builds (api-contract §4 `prs`), newest `updated_at` first.
fn join(prs_json: &str, releases_json: &str) -> anyhow::Result<Vec<Value>> {
    let prs: Vec<GhPr> = serde_json::from_str(prs_json)?;
    let releases: Vec<GhRelease> = serde_json::from_str(releases_json)?;
    let builds: std::collections::HashMap<u64, PrRelease> = releases.into_iter().filter_map(pr_release).collect();
    let mut rows: Vec<(i64, Value)> = prs
        .into_iter()
        .map(|pr| {
            let updated_at = timestamp(pr.updated_at.as_deref()).unwrap_or(0);
            let build = builds.get(&pr.number).map(|b| {
                json!({
                    "status": if b.sha == pr.head_ref_oid { "current" } else { "outdated" },
                    "version": b.version,
                    "sha": b.sha,
                    "published_at": b.published_at,
                })
            });
            let row = json!({
                "number": pr.number,
                "title": pr.title,
                "branch": pr.head_ref_name,
                "author": pr.author.map(|a| a.login).unwrap_or_default(),
                "url": pr.url,
                "draft": pr.is_draft,
                "head_sha": pr.head_ref_oid,
                "updated_at": updated_at,
                "build": build,
            });
            (updated_at, row)
        })
        .collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.0));
    Ok(rows.into_iter().map(|r| r.1).collect())
}

/// Why gh could not answer, as `error` (api-contract §4).
fn error_text(e: &GhError) -> String {
    match e {
        GhError::Missing => "gh not installed".into(),
        _ => e.msg(),
    }
}

/// The list object, from the two gh outputs or the reason there are none.
fn list_value(fetched_at: i64, outcome: Result<(String, String), String>) -> Value {
    let (error, prs) = match outcome {
        Err(e) => (Some(e), vec![]),
        Ok((p, r)) => match join(&p, &r) {
            Ok(rows) => (None, rows),
            Err(e) => (Some(format!("gh: unexpected output ({e})")), vec![]),
        },
    };
    json!({ "fetched_at": fetched_at, "error": error, "prs": prs })
}

/// `pr_builds_list`: two gh calls. Trouble with gh or the network is the `error` field, never
/// an `Err`. Run off the main thread.
pub fn list() -> Value {
    let outcome = (|| {
        match gh(&["auth", "status", "--hostname", "github.com"], VIEW_TIMEOUT) {
            Ok(_) => {}
            Err(GhError::Failed(_)) => return Err("gh not logged in".to_string()),
            Err(e) => return Err(error_text(&e)),
        }
        let prs = gh(
            &[
                "pr",
                "list",
                "--repo",
                REPO,
                "--state",
                "open",
                "--limit",
                "50",
                "--json",
                "number,title,headRefName,headRefOid,author,isDraft,updatedAt,url",
            ],
            VIEW_TIMEOUT,
        )
        .map_err(|e| error_text(&e))?;
        let releases =
            gh(&["api", &format!("repos/{REPO}/releases?per_page=100")], VIEW_TIMEOUT).map_err(|e| error_text(&e))?;
        Ok((prs, releases))
    })();
    list_value(jiff::Timestamp::now().as_second(), outcome)
}

pub enum FetchError {
    /// No `pr-<n>` release, or one without both assets.
    NoBuild,
    /// gh or the network failed: the text for `pr_install.error`.
    Gh(String),
}

/// The `pr-<n>` release as it is now.
pub fn fetch(number: u64) -> Result<PrRelease, FetchError> {
    let tag = format!("pr-{number}");
    let fields = format!("{RELEASE_FIELDS},isPrerelease");
    match gh(&["release", "view", &tag, "--repo", REPO, "--json", &fields], VIEW_TIMEOUT) {
        Ok(json) => parse_pr_release(&json, number).ok_or(FetchError::NoBuild),
        Err(GhError::Failed(m)) if m.contains("release not found") => Err(FetchError::NoBuild),
        Err(e) => Err(FetchError::Gh(error_text(&e))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIX: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/pr_builds");

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(std::path::Path::new(FIX).join(name)).unwrap()
    }

    fn rows() -> Vec<Value> {
        join(&fixture("pr_list.json"), &fixture("releases.json")).unwrap()
    }

    fn row(rows: &[Value], n: u64) -> &Value {
        rows.iter().find(|r| r["number"] == n).unwrap()
    }

    #[test]
    fn a_build_from_the_head_sha_is_current() {
        let rows = rows();
        let r = row(&rows, 12);
        assert_eq!(r["build"]["status"], "current");
        assert_eq!(r["build"]["version"], "0.1.57");
        assert_eq!(r["build"]["sha"], r["head_sha"]);
        assert_eq!(r["build"]["published_at"], 1793599500);
        assert_eq!(r["title"], "Add build on PRs");
        assert_eq!(r["branch"], "orch/add-build-pr/integration");
        assert_eq!(r["author"], "GuillaumeOj");
        assert_eq!(r["draft"], false);
        assert_eq!(r["updated_at"], 1793599000);
    }

    #[test]
    fn a_build_from_an_older_commit_is_outdated() {
        let rows = rows();
        let r = row(&rows, 14);
        assert_eq!(r["build"]["status"], "outdated");
        assert_eq!(r["build"]["version"], "0.1.61");
        assert_ne!(r["build"]["sha"], r["head_sha"]);
    }

    #[test]
    fn no_release_or_an_incomplete_one_is_no_build() {
        let rows = rows();
        // #15 has no release; #16's release has no .sig; #17's tag is not a prerelease.
        for n in [15, 16, 17] {
            assert!(row(&rows, n)["build"].is_null(), "PR #{n}");
        }
        assert!(row(&rows, 15)["draft"].as_bool().unwrap());
    }

    #[test]
    fn rows_are_newest_updated_first_and_only_open_prs_appear() {
        let rows = rows();
        let times: Vec<i64> = rows.iter().map(|r| r["updated_at"].as_i64().unwrap()).collect();
        assert!(times.windows(2).all(|w| w[0] >= w[1]), "{times:?}");
        // A release for a PR that is not in the list (closed) does not show up.
        assert_eq!(rows.len(), 5);
        assert!(rows.iter().all(|r| r["number"] != 99));
    }

    #[test]
    fn the_main_releases_in_the_same_listing_are_ignored() {
        let rels: Vec<GhRelease> = serde_json::from_str(&fixture("releases.json")).unwrap();
        let found: Vec<u64> = rels.into_iter().filter_map(pr_release).map(|(n, _)| n).collect();
        assert_eq!(
            {
                let mut f = found;
                f.sort();
                f
            },
            vec![12, 14, 99]
        );
    }

    #[test]
    fn pr_tags() {
        assert_eq!(pr_number("pr-12"), Some(12));
        for t in ["pr-", "pr-0", "pr-012", "pr-1a", "v0.1.57", "pr-12-x", "xpr-12"] {
            assert_eq!(pr_number(t), None, "{t}");
        }
    }

    #[test]
    fn release_view_output_parses_and_must_match_the_number() {
        let json = fixture("release_view.json");
        let rel = parse_pr_release(&json, 12).unwrap();
        assert_eq!(rel.tag, "pr-12");
        assert_eq!(rel.version, "0.1.57");
        assert_eq!(rel.sha, "f9b1e20c0ffeef9b1e20c0ffeef9b1e20c0ffee0");
        assert_eq!(rel.title, "Add build on PRs");
        assert_eq!(rel.tarball, "Orchestrator_0.1.57_aarch64.app.tar.gz");
        assert_eq!(rel.sig, "Orchestrator_0.1.57_aarch64.app.tar.gz.sig");
        assert_eq!(rel.tarball_size, 104857600);
        assert!(parse_pr_release(&json, 13).is_none());
        assert!(parse_pr_release("not json", 12).is_none());
    }

    #[test]
    fn a_release_view_without_both_assets_or_not_a_prerelease_is_no_build() {
        let mut v: Value = serde_json::from_str(&fixture("release_view.json")).unwrap();
        v["isPrerelease"] = false.into();
        assert!(parse_pr_release(&v.to_string(), 12).is_none());
        v["isPrerelease"] = true.into();
        v["assets"].as_array_mut().unwrap().pop();
        assert!(parse_pr_release(&v.to_string(), 12).is_none());
    }

    #[test]
    fn gh_trouble_is_an_error_text_with_no_rows() {
        let v = list_value(7, Err("offline".into()));
        assert_eq!(v["error"], "offline");
        assert_eq!(v["prs"], json!([]));
        assert_eq!(v["fetched_at"], 7);
        let v = list_value(7, Ok(("{".into(), "[]".into())));
        assert!(v["error"].as_str().unwrap().starts_with("gh: unexpected output"));
        let v = list_value(7, Ok((fixture("pr_list.json"), fixture("releases.json"))));
        assert!(v["error"].is_null());
        assert_eq!(v["prs"].as_array().unwrap().len(), 5);
        assert_eq!(error_text(&GhError::Missing), "gh not installed");
        assert_eq!(error_text(&GhError::Offline), "offline");
        assert_eq!(error_text(&GhError::Failed("boom".into())), "gh: boom");
    }
}
