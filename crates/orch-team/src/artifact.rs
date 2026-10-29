//! What a hand-off URI points at (FR-602): a Penpot or Figma frame link, a
//! repo-relative exported image, or another link. Pure: whether a file is on
//! the integration branch is the runtime's to say (`on_integration`).

use serde::{Deserialize, Serialize};

/// api-contract §3 `Artifact.meta`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactMeta {
    /// `penpot`, `figma`, `file` or `url`.
    pub tool: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default)]
    pub previewable: bool,
    /// A file: present on the integration branch now (filled by the view).
    #[serde(default)]
    pub on_integration: Option<bool>,
}

/// Image types a preview can show.
pub const PREVIEWABLE: &[(&str, &str)] = &[
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("webp", "image/webp"),
    ("svg", "image/svg+xml"),
];

/// The MIME type of a previewable path.
pub fn image_mime(path: &str) -> Option<&'static str> {
    let ext = path.rsplit_once('.')?.1.to_ascii_lowercase();
    PREVIEWABLE.iter().find(|(e, _)| *e == ext).map(|(_, m)| *m)
}

/// The MIME type of an image a feature doc can show: the previewable ones
/// and GIF (not previewed in the Design section).
pub fn doc_image_mime(path: &str) -> Option<&'static str> {
    image_mime(path).or_else(|| path.rsplit_once('.')?.1.eq_ignore_ascii_case("gif").then_some("image/gif"))
}

fn percent_decode(s: &str) -> String {
    let hex = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match (b[i], b.get(i + 1).copied().and_then(hex), b.get(i + 2).copied().and_then(hex)) {
            (b'%', Some(h), Some(l)) => {
                out.push(h * 16 + l);
                i += 3;
            }
            (b'+', ..) => {
                out.push(b' ');
                i += 1;
            }
            (c, ..) => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `a=1&b=2` → pairs (decoded).
fn query_pairs(q: &str) -> Vec<(String, String)> {
    q.split('&')
        .filter(|p| !p.is_empty())
        .map(|p| match p.split_once('=') {
            Some((k, v)) => (percent_decode(k), percent_decode(v)),
            None => (percent_decode(p), String::new()),
        })
        .collect()
}

struct Url<'a> {
    host: &'a str,
    /// Path segments, decoded.
    path: Vec<String>,
    query: Vec<(String, String)>,
    /// The fragment as a path + query (Penpot routes live there: `#/workspace?file-id=…`).
    frag_path: Vec<String>,
    frag_query: Vec<(String, String)>,
}

fn split_path_query(s: &str) -> (Vec<String>, Vec<(String, String)>) {
    let (p, q) = s.split_once('?').unwrap_or((s, ""));
    (p.split('/').filter(|x| !x.is_empty()).map(percent_decode).collect(), query_pairs(q))
}

fn parse_url(uri: &str) -> Option<Url<'_>> {
    let rest = uri.strip_prefix("https://").or_else(|| uri.strip_prefix("http://"))?;
    let (before_frag, frag) = rest.split_once('#').unwrap_or((rest, ""));
    let host_end = before_frag.find(['/', '?']).unwrap_or(before_frag.len());
    let host = before_frag[..host_end].rsplit('@').next().unwrap_or("");
    let (path, query) = split_path_query(&before_frag[host_end..]);
    let (frag_path, frag_query) = split_path_query(frag);
    Some(Url { host, path, query, frag_path, frag_query })
}

impl Url<'_> {
    fn param(&self, keys: &[&str]) -> Option<String> {
        self.frag_query
            .iter()
            .chain(&self.query)
            .find(|(k, v)| keys.contains(&k.as_str()) && !v.is_empty())
            .map(|(_, v)| v.clone())
    }
}

fn penpot(u: &Url) -> ArtifactMeta {
    // Old routes: `#/workspace/<project>/<file>?page-id=…`, `#/view/<file>?…`.
    let seg_file = match u.frag_path.first().map(String::as_str) {
        Some("workspace") => u.frag_path.get(2).cloned(),
        Some("view") => u.frag_path.get(1).cloned(),
        _ => None,
    };
    ArtifactMeta {
        tool: "penpot".into(),
        file_id: u.param(&["file-id"]).or(seg_file),
        page_id: u.param(&["page-id"]),
        frame_id: u.param(&["frame-id", "board-id"]),
        ..Default::default()
    }
}

fn figma(u: &Url) -> Option<ArtifactMeta> {
    // `/(design|file|proto|board|slides)/<key>/<name>` or `/design/<key>/branch/<branch>/<name>`.
    let kinds = ["design", "file", "proto", "board", "slides"];
    let i = u.path.iter().position(|s| kinds.contains(&s.as_str()))?;
    let mut key = u.path.get(i + 1)?.clone();
    let mut name = u.path.get(i + 2).cloned();
    if name.as_deref() == Some("branch") {
        key = u.path.get(i + 3)?.clone();
        name = u.path.get(i + 4).cloned();
    }
    Some(ArtifactMeta {
        tool: "figma".into(),
        title: name.map(|n| n.replace('-', " ")).filter(|n| !n.trim().is_empty()),
        file_key: Some(key),
        node_id: u.param(&["node-id"]).map(|n| n.replace('-', ":")),
        ..Default::default()
    })
}

/// A repo-relative path: no scheme, not absolute, never leaving the repo.
pub fn repo_path(uri: &str) -> Option<String> {
    let p = uri.trim().trim_start_matches("./");
    if p.is_empty() || p.starts_with('/') || p.starts_with('~') || p.contains("://") || p.contains('\\') {
        return None;
    }
    if p.split('/').any(|s| s == ".." || s.is_empty()) || p.contains(['\n', '\0']) {
        return None;
    }
    Some(p.to_string())
}

/// Parses a hand-off URI. `None` for what is neither a link nor a repo path.
pub fn parse(uri: &str) -> Option<ArtifactMeta> {
    let uri = uri.trim();
    if let Some(u) = parse_url(uri) {
        let host = u.host.to_ascii_lowercase();
        if host == "penpot.app" || host.ends_with(".penpot.app") {
            return Some(penpot(&u));
        }
        if (host == "figma.com" || host.ends_with(".figma.com"))
            && let Some(m) = figma(&u)
        {
            return Some(m);
        }
        return Some(ArtifactMeta { tool: "url".into(), title: Some(host), ..Default::default() });
    }
    let path = repo_path(uri)?;
    Some(ArtifactMeta {
        tool: "file".into(),
        title: path.rsplit('/').next().map(str::to_string),
        previewable: image_mime(&path).is_some(),
        path: Some(path),
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn penpot_links_new_and_old_routes() {
        let m =
            parse("https://design.penpot.app/#/workspace?team-id=t1&file-id=f-123&page-id=p-9&board-id=b-7").unwrap();
        assert_eq!(m.tool, "penpot");
        assert_eq!(
            (m.file_id.as_deref(), m.page_id.as_deref(), m.frame_id.as_deref()),
            (Some("f-123"), Some("p-9"), Some("b-7"))
        );
        let m = parse("https://design.penpot.app/#/workspace/proj-1/file-2?page-id=page-3").unwrap();
        assert_eq!((m.file_id.as_deref(), m.page_id.as_deref(), m.frame_id), (Some("file-2"), Some("page-3"), None));
        assert_eq!(parse("https://penpot.example.com/#/view/file-5").unwrap().tool, "url", "only penpot.app hosts");
        let m = parse("https://design.penpot.app/#/view/file-5?page-id=p&frame-id=fr&section=interactions").unwrap();
        assert_eq!((m.file_id.as_deref(), m.frame_id.as_deref()), (Some("file-5"), Some("fr")));
        assert!(!m.previewable);
    }

    #[test]
    fn figma_links() {
        let m = parse("https://www.figma.com/design/AbC123/Shop-Checkout?node-id=12-34&t=x").unwrap();
        assert_eq!(m.tool, "figma");
        assert_eq!(
            (m.file_key.as_deref(), m.node_id.as_deref(), m.title.as_deref()),
            (Some("AbC123"), Some("12:34"), Some("Shop Checkout"))
        );
        let m = parse("https://figma.com/design/Main1/branch/Br2/Name?node-id=1%3A2").unwrap();
        assert_eq!((m.file_key.as_deref(), m.node_id.as_deref()), (Some("Br2"), Some("1:2")));
        let m = parse("https://www.figma.com/file/K9/Old-file").unwrap();
        assert_eq!((m.file_key.as_deref(), m.node_id), (Some("K9"), None));
        // Not a file link: a plain URL.
        assert_eq!(parse("https://www.figma.com/community").unwrap().tool, "url");
    }

    #[test]
    fn repo_files_and_other_urls() {
        let m = parse(".orchestrator/features/shop/design/checkout.png").unwrap();
        assert_eq!(m.tool, "file");
        assert_eq!(m.path.as_deref(), Some(".orchestrator/features/shop/design/checkout.png"));
        assert_eq!(m.title.as_deref(), Some("checkout.png"));
        assert!(m.previewable);
        assert!(parse("./design/a.SVG").unwrap().previewable);
        assert!(parse("design/a.webp").unwrap().previewable);
        assert!(!parse("docs/spec.md").unwrap().previewable);
        for bad in ["/etc/passwd", "../x.png", "a/../../x.png", "~/x.png", "a//b.png", ""] {
            assert_eq!(parse(bad), None, "{bad}");
        }
        let m = parse("https://example.com/x?y=1").unwrap();
        assert_eq!((m.tool.as_str(), m.title.as_deref()), ("url", Some("example.com")));
        assert_eq!(image_mime("a/b.JPG"), Some("image/jpeg"));
        assert_eq!(image_mime("a/b.gif"), None);
        assert_eq!(doc_image_mime("a/b.GIF"), Some("image/gif"));
        assert_eq!(doc_image_mime("a/b.svg"), Some("image/svg+xml"));
        assert_eq!(doc_image_mime("a/b.txt"), None);
    }

    #[test]
    fn wire_shape() {
        let v = serde_json::to_value(parse("design/a.png").unwrap()).unwrap();
        assert_eq!(
            v,
            serde_json::json!({"tool": "file", "title": "a.png", "path": "design/a.png", "previewable": true, "on_integration": null})
        );
        assert_eq!(percent_decode("a%20b%2"), "a b%2");
    }
}
