//! FR-505 (C): publish a feature's docs to Notion and bring comments back.
//!
//! One Notion page per feature under the project's `notion_parent_page`,
//! with one child page per doc (`spec.md` first), made from the copy on the
//! **integration branch** (FR-504). Publishing again refreshes the same
//! pages (their blocks are replaced). Comments on those pages are polled
//! (every 15 min while the feature has an active run, and on demand with
//! `notion_status`) and become Inbox items. The core is Notion's only client:
//! the token is the Keychain's `notion` account, sent in a header on curl's
//! stdin (`net.rs`). The transport is a trait ([`NotionApi`]); tests use
//! [`fake::FakeNotion`].

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use orch_protocol::now_ms;
use orch_secrets::Secrets;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::integrations::NOTION_VERSION;
use crate::net::{HttpRequest, Net};
use crate::runtime::Runtime;

pub const API: &str = "https://api.notion.com";
/// Settings key prefix of a feature's publication state.
pub const STATE_KEY: &str = "notion.feature.";
/// How often comments are read while the feature is active.
pub const POLL_EVERY_MS: u64 = 15 * 60 * 1000;
/// Notion takes at most this many blocks per request, and this many chars per text.
const MAX_BLOCKS: usize = 100;
const MAX_TEXT: usize = 2000;
/// … and this many rich-text runs per block.
const MAX_RUNS: usize = 100;
/// Retries of a rate-limited call (Retry-After, at most this long each).
const RETRIES: u32 = 3;
const MAX_RETRY_WAIT: Duration = Duration::from_secs(10);

/// One Notion API call; returns the JSON answer.
pub trait NotionApi: Send + Sync {
    fn call(&self, method: &'static str, path: &str, body: Option<&Value>) -> Result<Value>;
}

/// Tests replace the transport.
#[derive(Clone)]
pub struct NotionFactory(pub Arc<dyn Fn() -> Box<dyn NotionApi> + Send + Sync>);

impl std::fmt::Debug for NotionFactory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NotionFactory")
    }
}

/// A request to api.notion.com with the integration token (the publish's
/// and the integration test's).
pub fn request(
    token: &str,
    method: &'static str,
    path: &str,
    body: Option<&Value>,
    timeout: Duration,
    max_body: usize,
) -> HttpRequest {
    let mut headers =
        vec![("Authorization".into(), format!("Bearer {token}")), ("Notion-Version".into(), NOTION_VERSION.into())];
    if body.is_some() {
        headers.push(("Content-Type".into(), "application/json".into()));
    }
    HttpRequest { method, url: format!("{API}{path}"), headers, body: body.map(Value::to_string), timeout, max_body }
}

/// The real transport: one per operation (a publish, a poll), which reads
/// the token from the Keychain once.
pub struct NotionHttp {
    pub net: Arc<dyn Net>,
    pub secrets: Arc<dyn Secrets>,
    pub token: std::sync::OnceLock<orch_secrets::Secret>,
}

impl NotionHttp {
    fn token(&self) -> Result<&orch_secrets::Secret> {
        if let Some(t) = self.token.get() {
            return Ok(t);
        }
        let t =
            self.secrets.get("notion")?.ok_or_else(|| anyhow!("no Notion token: add it in Settings › Integrations"))?;
        Ok(self.token.get_or_init(|| t))
    }
}

impl NotionApi for NotionHttp {
    fn call(&self, method: &'static str, path: &str, body: Option<&Value>) -> Result<Value> {
        let req =
            request(self.token()?.expose(), method, path, body, Duration::from_secs(30), crate::net::API_MAX_BODY);
        let r = http_with_retry(self.net.as_ref(), &req, std::thread::sleep)?;
        let v: Value = serde_json::from_str(&r.body).unwrap_or(Value::Null);
        if !(200..300).contains(&r.status) {
            let msg = v["message"].as_str().unwrap_or("").to_string();
            match r.status {
                401 | 403 => bail!("Notion: not authorized ({msg})"),
                404 => bail!("Notion: not found ({msg}); share the page with the integration"),
                429 => bail!("Notion rate limit reached: try again later"),
                s => bail!("Notion: {s} {msg}"),
            }
        }
        Ok(v)
    }
}

/// One request, retried (up to [`RETRIES`]) after a 429 once its Retry-After
/// (at most [`MAX_RETRY_WAIT`], else 1 s) has passed.
pub fn http_with_retry(net: &dyn Net, req: &HttpRequest, sleep: impl Fn(Duration)) -> Result<crate::net::HttpResponse> {
    let mut r = net.http(req)?;
    for _ in 0..RETRIES {
        if r.status != 429 {
            break;
        }
        let wait = r.header("retry-after").and_then(|s| s.trim().parse::<f64>().ok()).filter(|w| w.is_finite());
        // Clamped before the conversion: `from_secs_f64` panics on a huge value.
        sleep(Duration::from_secs_f64(wait.unwrap_or(1.0).clamp(0.0, MAX_RETRY_WAIT.as_secs_f64())));
        r = net.http(req)?;
    }
    Ok(r)
}

/// A feature's publication, kept in the settings.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Published {
    pub page_id: Option<String>,
    pub page_url: Option<String>,
    /// Doc name → its child page id.
    #[serde(default)]
    pub docs: BTreeMap<String, String>,
    pub published_at_ms: Option<u64>,
    /// Comment ids already turned into Inbox items.
    #[serde(default)]
    pub comments_seen: Vec<String>,
    #[serde(default)]
    pub last_poll_ms: u64,
    pub error: Option<String>,
}

/// A Notion page id from a URL or an id (32 hex digits, dashes or not).
pub fn page_id(s: &str) -> Option<String> {
    // `?pvs=4`, `?source=copy_link`, `#heading` are not part of the id.
    let s = s.trim().split(['?', '#']).next().unwrap_or("");
    let hex: String = s.trim().trim_end_matches('/').rsplit(['/', '-']).next().unwrap_or("").chars().collect();
    let hex = if hex.len() == 32 { hex } else { s.chars().filter(|c| c.is_ascii_hexdigit()).collect::<String>() };
    let hex = hex.get(hex.len().saturating_sub(32)..)?.to_string();
    (hex.len() == 32 && hex.chars().all(|c| c.is_ascii_hexdigit())).then_some(hex)
}

// ------------------------------------------------------------ markdown → blocks

fn text(s: &str, link: Option<&str>, code: bool) -> Vec<Value> {
    let chars: Vec<char> = s.chars().collect();
    chars
        .chunks(MAX_TEXT)
        .map(|c| {
            let content: String = c.iter().collect();
            let mut t = json!({"type": "text", "text": {"content": content}});
            if let Some(u) = link {
                t["text"]["link"] = json!({"url": u});
            }
            if code {
                t["annotations"] = json!({"code": true});
            }
            t
        })
        .collect()
}

/// Inline markdown: `[text](url)` links and `code`; the rest is plain text.
pub fn rich_text(line: &str) -> Vec<Value> {
    let mut out = vec![];
    let mut rest = line;
    while !rest.is_empty() {
        let link = rest.find('[').and_then(|i| {
            let close = rest[i..].find("](")? + i;
            let end = rest[close..].find(')')? + close;
            Some((i, close, end))
        });
        let code = rest.find('`').and_then(|i| Some((i, rest[i + 1..].find('`')? + i + 1)));
        match (link, code) {
            (Some((i, close, end)), c) if c.is_none_or(|(ci, _)| i < ci) => {
                out.extend(text(&rest[..i], None, false));
                let url = &rest[close + 2..end];
                let ok = url.starts_with("http://") || url.starts_with("https://");
                out.extend(text(&rest[i + 1..close], ok.then_some(url), false));
                rest = &rest[end + 1..];
            }
            (_, Some((i, end))) => {
                out.extend(text(&rest[..i], None, false));
                out.extend(text(&rest[i + 1..end], None, true));
                rest = &rest[end + 1..];
            }
            _ => {
                out.extend(text(rest, None, false));
                rest = "";
            }
        }
    }
    out.retain(|t| t["text"]["content"].as_str().is_some_and(|c| !c.is_empty()));
    out
}

/// Notion takes at most 100 rich-text runs per block: past that, the text is
/// sent plain (in 2000-char runs), and cut at 100 runs.
fn cap_runs(rt: Vec<Value>) -> Vec<Value> {
    if rt.len() <= MAX_RUNS {
        return rt;
    }
    let plain: String = rt.iter().filter_map(|t| t["text"]["content"].as_str()).collect();
    let mut out = text(&plain, None, false);
    out.truncate(MAX_RUNS);
    out
}

fn block(kind: &str, rt: Vec<Value>) -> Value {
    json!({"object": "block", "type": kind, kind: {"rich_text": cap_runs(rt)}})
}

fn code_block(code: &str, lang: &str) -> Value {
    // Notion knows a fixed list of languages; anything else is plain text.
    const KNOWN: &[&str] = &[
        "rust",
        "typescript",
        "javascript",
        "json",
        "bash",
        "shell",
        "python",
        "html",
        "css",
        "sql",
        "yaml",
        "markdown",
    ];
    let lang = match lang {
        "ts" => "typescript",
        "js" => "javascript",
        "sh" | "zsh" => "shell",
        l if KNOWN.contains(&l) => l,
        _ => "plain text",
    };
    // At most 100 runs per block, like the others: past that the code is cut.
    let mut rt = text(code, None, false);
    rt.truncate(MAX_RUNS);
    json!({"object": "block", "type": "code", "code": {"rich_text": rt, "language": lang}})
}

fn table(rows: &[Vec<String>]) -> Value {
    let width = rows.iter().map(Vec::len).max().unwrap_or(1).max(1);
    let children: Vec<Value> = rows
        .iter()
        .map(|r| {
            let cells: Vec<Value> =
                (0..width).map(|i| json!(cap_runs(rich_text(r.get(i).map(String::as_str).unwrap_or(""))))).collect();
            json!({"object": "block", "type": "table_row", "table_row": {"cells": cells}})
        })
        .collect();
    json!({"object": "block", "type": "table", "table": {"table_width": width, "has_column_header": true,
           "has_row_header": false, "children": children}})
}

fn cells(line: &str) -> Vec<String> {
    let t = line.trim().trim_start_matches('|').trim_end_matches('|');
    t.split('|').map(|c| c.trim().to_string()).collect()
}

/// Markdown → Notion blocks: headings, paragraphs, lists, code, tables and
/// links; anything else (HTML, quotes, rules…) as a plain code block.
pub fn blocks(md: &str) -> Vec<Value> {
    let lines: Vec<&str> = md.lines().collect();
    let mut out = vec![];
    let mut para: Vec<&str> = vec![];
    let flush = |para: &mut Vec<&str>, out: &mut Vec<Value>| {
        if !para.is_empty() {
            out.push(block("paragraph", rich_text(&para.join(" "))));
            para.clear();
        }
    };
    let mut i = 0;
    while i < lines.len() {
        let l = lines[i];
        let t = l.trim();
        if let Some(lang) = t.strip_prefix("```") {
            flush(&mut para, &mut out);
            let mut code = vec![];
            i += 1;
            while i < lines.len() && !lines[i].trim_start().starts_with("```") {
                code.push(lines[i]);
                i += 1;
            }
            out.push(code_block(&code.join("\n"), lang.trim()));
        } else if t.is_empty() {
            flush(&mut para, &mut out);
        } else if let Some(h) = ["### ", "## ", "# "].iter().position(|p| t.starts_with(p)) {
            flush(&mut para, &mut out);
            let kind = ["heading_3", "heading_2", "heading_1"][h];
            out.push(block(kind, rich_text(t.trim_start_matches('#').trim())));
        } else if t.starts_with('|') {
            flush(&mut para, &mut out);
            let mut rows = vec![];
            while i < lines.len() && lines[i].trim().starts_with('|') {
                let r = cells(lines[i]);
                if !r.iter().all(|c| !c.is_empty() && c.chars().all(|ch| "-: ".contains(ch))) {
                    rows.push(r);
                }
                i += 1;
            }
            // Notion takes at most 100 rows per table: the rest in more
            // tables, each with the header row again.
            let (head, body) = rows.split_first().map_or((None, &rows[..]), |(h, b)| (Some(h), b));
            for chunk in body.chunks(MAX_BLOCKS - 1) {
                let mut t: Vec<Vec<String>> = head.cloned().into_iter().collect();
                t.extend(chunk.iter().cloned());
                out.push(table(&t));
            }
            if body.is_empty() {
                out.push(table(&rows));
            }
            continue;
        } else if let Some(item) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")) {
            flush(&mut para, &mut out);
            out.push(block("bulleted_list_item", rich_text(item)));
        } else if let Some(item) =
            t.split_once(". ").filter(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())).map(|(_, r)| r)
        {
            flush(&mut para, &mut out);
            out.push(block("numbered_list_item", rich_text(item)));
        } else if t.starts_with('<') || t.starts_with("> ") || t == "---" {
            flush(&mut para, &mut out);
            out.push(code_block(l, ""));
        } else {
            para.push(t);
        }
        i += 1;
    }
    flush(&mut para, &mut out);
    out
}

fn title_props(title: &str) -> Value {
    json!({"title": {"title": text(title, None, false)}})
}

/// Replaces a page's blocks with `children`. Its child pages (the feature
/// page's docs are `child_page` blocks of it) stay: deleting one would
/// archive that doc's page.
fn replace_children(api: &dyn NotionApi, page: &str, children: &[Value]) -> Result<()> {
    let mut cursor: Option<String> = None;
    let mut old = vec![];
    for _ in 0..100 {
        let q = cursor.as_deref().map(|c| format!("&start_cursor={c}")).unwrap_or_default();
        let r = api.call("GET", &format!("/v1/blocks/{page}/children?page_size=100{q}"), None)?;
        old.extend(
            r["results"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|b| !matches!(b["type"].as_str(), Some("child_page" | "child_database")))
                .filter_map(|b| b["id"].as_str().map(str::to_string)),
        );
        if r["has_more"] != true {
            break;
        }
        cursor = r["next_cursor"].as_str().map(str::to_string);
    }
    for b in old {
        api.call("DELETE", &format!("/v1/blocks/{b}"), None)?;
    }
    for chunk in children.chunks(MAX_BLOCKS) {
        api.call("PATCH", &format!("/v1/blocks/{page}/children"), Some(&json!({"children": chunk})))?;
    }
    Ok(())
}

/// Creates a page under `parent` with the first 100 of `children`; returns
/// `(id, url)`. The caller records the id before [`append_rest`], so a failed
/// append never leads to a second page.
fn create_page(api: &dyn NotionApi, parent: &str, title: &str, children: &[Value]) -> Result<(String, Option<String>)> {
    let first: Vec<Value> = children.iter().take(MAX_BLOCKS).cloned().collect();
    let page = api.call(
        "POST",
        "/v1/pages",
        Some(&json!({"parent": {"page_id": parent}, "properties": title_props(title), "children": first})),
    )?;
    let id = page["id"].as_str().ok_or_else(|| anyhow!("Notion: no page id in the answer"))?.to_string();
    Ok((id, page["url"].as_str().map(str::to_string)))
}

/// The blocks past the first 100 of a page just created.
fn append_rest(api: &dyn NotionApi, id: &str, children: &[Value]) -> Result<()> {
    for chunk in children.chunks(MAX_BLOCKS).skip(1) {
        api.call("PATCH", &format!("/v1/blocks/{id}/children"), Some(&json!({"children": chunk})))?;
    }
    Ok(())
}

/// A comment as an Inbox item's payload.
fn comment_payload(c: &Value, page_url: &str) -> Value {
    let text: String =
        c["rich_text"].as_array().into_iter().flatten().filter_map(|t| t["plain_text"].as_str()).collect();
    let author = c["created_by"]["name"].as_str().or(c["created_by"]["id"].as_str()).unwrap_or("someone");
    json!({"kind": "question", "source": "notion", "page_url": page_url, "author": author, "text": text,
           "comment": c["id"]})
}

impl Runtime {
    pub(crate) fn notion_api(&self) -> Box<dyn NotionApi> {
        match &self.cfg.notion {
            Some(f) => (f.0)(),
            None => Box::new(NotionHttp {
                net: self.cfg.net.clone(),
                secrets: self.cfg.secrets.clone(),
                token: std::sync::OnceLock::new(),
            }),
        }
    }

    fn published(&self, feature: &str) -> Published {
        self.db
            .setting(&format!("{STATE_KEY}{feature}"))
            .ok()
            .flatten()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default()
    }

    fn save_published(&self, feature: &str, p: &Published) -> Result<()> {
        self.db.set_setting(&format!("{STATE_KEY}{feature}"), &serde_json::to_value(p)?)
    }

    /// `notion_publish`: creates (then refreshes) the feature's page and one
    /// child page per doc, from the integration branch.
    /// One publish or comment poll of a feature at a time: both read, change
    /// and save its state (a concurrent one would lose comments it saw, and
    /// make their Inbox items twice).
    fn notion_lock(&self, feature_id: &str) -> Arc<std::sync::Mutex<()>> {
        self.notion_locks.lock().unwrap_or_else(|e| e.into_inner()).entry(feature_id.to_string()).or_default().clone()
    }

    pub fn notion_publish(&self, feature_id: &str) -> Result<Value> {
        let lock = self.notion_lock(feature_id);
        let _held = lock.lock().unwrap_or_else(|e| e.into_inner());
        let f = self.db.feature(feature_id)?.ok_or_else(|| anyhow!("no feature {feature_id}"))?;
        let project = self.db.project(&f.project_id)?.ok_or_else(|| anyhow!("no project {}", f.project_id))?;
        let parent_raw = project.typed_settings().notion_parent_page.clone().unwrap_or_default();
        let parent = page_id(&parent_raw)
            .ok_or_else(|| anyhow!("set the project's Notion parent page in its settings first"))?;
        let wt =
            f.integration_worktree.clone().ok_or_else(|| anyhow!("feature {} has no integration worktree", f.slug))?;
        let names = crate::team::feature_docs(&wt, &f.slug);
        if names.is_empty() {
            bail!("feature {} has no docs on its integration branch", f.slug);
        }
        let api = self.notion_api();
        let mut p = self.published(feature_id);
        let r = (|| -> Result<()> {
            let intro = blocks(&format!(
                "Published by Orchestrator from the integration branch of `{}`. Comment on these pages \
                 (page comments, at the top of a page): they come back to the app's Inbox.",
                f.slug
            ));
            match &p.page_id {
                Some(id) => replace_children(api.as_ref(), id, &intro)?,
                None => {
                    let (id, url) = create_page(api.as_ref(), &parent, &f.title, &intro)?;
                    (p.page_id, p.page_url) = (Some(id.clone()), url);
                    p.docs.clear();
                    self.save_published(feature_id, &p)?;
                    append_rest(api.as_ref(), &id, &intro)?;
                }
            }
            let root = p.page_id.clone().ok_or_else(|| anyhow!("no page"))?;
            for n in &names {
                let md = crate::team::read_feature_doc(&wt, &f.slug, n)?;
                let bl = blocks(&md);
                match p.docs.get(n) {
                    Some(id) => replace_children(api.as_ref(), id, &bl).with_context(|| n.clone())?,
                    None => {
                        let (id, _) = create_page(api.as_ref(), &root, n, &bl).with_context(|| n.clone())?;
                        p.docs.insert(n.clone(), id.clone());
                        self.save_published(feature_id, &p)?;
                        append_rest(api.as_ref(), &id, &bl).with_context(|| n.clone())?;
                    }
                }
            }
            Ok(())
        })();
        match r {
            Ok(()) => {
                p.published_at_ms = Some(now_ms());
                p.error = None;
                self.save_published(feature_id, &p)?;
                self.event("-", "notion_published", json!({"feature": feature_id, "docs": names}));
                Ok(json!({"page_url": p.page_url, "published_at_ms": p.published_at_ms, "docs": names}))
            }
            Err(e) => {
                p.error = Some(format!("{e:#}"));
                self.save_published(feature_id, &p)?;
                Err(e)
            }
        }
    }

    /// Reads the comments of a published feature's pages; new ones become
    /// Inbox items (question kind, level 3, `open` → the page). Returns how many.
    pub fn notion_poll(&self, feature_id: &str) -> Result<usize> {
        let lock = self.notion_lock(feature_id);
        let _held = lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut p = self.published(feature_id);
        let Some(root) = p.page_id.clone() else { return Ok(0) };
        let api = self.notion_api();
        let url = p.page_url.clone().unwrap_or_default();
        let mut new = 0;
        let pages: Vec<String> = std::iter::once(root).chain(p.docs.values().cloned()).collect();
        let r = (|| -> Result<()> {
            for page in &pages {
                // Page comments only (the discussion at the top of a page):
                // reading every block's would cost a request per block.
                let mut list = vec![];
                let mut cursor: Option<String> = None;
                for _ in 0..100 {
                    let q = cursor.as_deref().map(|c| format!("&start_cursor={c}")).unwrap_or_default();
                    let c = api.call("GET", &format!("/v1/comments?block_id={page}&page_size=100{q}"), None)?;
                    list.extend(c["results"].as_array().cloned().unwrap_or_default());
                    if c["has_more"] != true {
                        break;
                    }
                    cursor = c["next_cursor"].as_str().map(str::to_string);
                }
                for cm in &list {
                    let Some(id) = cm["id"].as_str() else { continue };
                    if p.comments_seen.iter().any(|s| s == id) {
                        continue;
                    }
                    let payload = comment_payload(cm, &url);
                    let body = format!(
                        "Notion comment from {}\n{}",
                        payload["author"].as_str().unwrap_or(""),
                        payload["text"].as_str().unwrap_or("")
                    );
                    self.store.write(|s| s.add_inbox_item(None, "notion", body, payload, 0))?;
                    p.comments_seen.push(id.to_string());
                    new += 1;
                }
            }
            Ok(())
        })();
        p.last_poll_ms = now_ms();
        p.error = r.as_ref().err().map(|e| format!("{e:#}"));
        self.save_published(feature_id, &p)?;
        r.map(|_| new)
    }

    /// `notion_status`: reads the comments when the last read is over a
    /// minute old (on demand), then reports.
    pub fn notion_status(&self, feature_id: &str) -> Result<Value> {
        let p = self.published(feature_id);
        if p.page_id.is_some() && now_ms().saturating_sub(p.last_poll_ms) > 60_000 {
            let _ = self.notion_poll(feature_id);
        }
        let p = self.published(feature_id);
        Ok(json!({"page_url": p.page_url, "published_at_ms": p.published_at_ms,
                  "comments_seen": p.comments_seen.len(), "error": p.error}))
    }

    /// Housekeeping: at most once a minute, a thread of its own polls the
    /// features with an active run and a published page every 15 min (the
    /// calls block; housekeeping must not).
    pub(crate) fn notion_tick(&self) {
        let now = now_ms();
        {
            let mut h = self.house();
            if now.saturating_sub(h.last_notion_ms) < 60_000 {
                return;
            }
            h.last_notion_ms = now;
        }
        let Some(me) = self.me.upgrade() else { return };
        if self.notion_polling.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return; // the last pass still runs
        }
        std::thread::spawn(move || {
            me.notion_poll_due(now);
            me.notion_polling.store(false, std::sync::atomic::Ordering::SeqCst);
        });
    }

    fn notion_poll_due(&self, now: u64) {
        for run in self.db.runs(None, true).unwrap_or_default() {
            let p = self.published(&run.feature_id);
            if p.page_id.is_some() && now.saturating_sub(p.last_poll_ms) >= POLL_EVERY_MS {
                if let Err(e) = self.notion_poll(&run.feature_id) {
                    self.event("-", "notion_poll_error", json!({"feature": run.feature_id, "error": format!("{e:#}")}));
                }
            }
        }
    }
}

/// Notion in memory, for tests: pages with blocks, and comments.
pub mod fake {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    use anyhow::{Result, bail};
    use serde_json::{Value, json};

    use super::NotionApi;

    #[derive(Default)]
    pub struct World {
        /// id → (parent, title, blocks).
        pub pages: BTreeMap<String, (String, String, Vec<Value>)>,
        /// (id, page, author, text).
        pub comments: Vec<(String, String, String, String)>,
        pub calls: Vec<String>,
        pub next: u64,
        /// The next PATCH (block append) fails this many times.
        pub fail_patch: u32,
    }

    #[derive(Clone, Default)]
    pub struct FakeNotion(pub Arc<Mutex<World>>);

    impl FakeNotion {
        pub fn comment(&self, page: &str, author: &str, text: &str) {
            let mut w = self.0.lock().unwrap();
            w.next += 1;
            let id = format!("c{}", w.next);
            w.comments.push((id, page.into(), author.into(), text.into()));
        }

        pub fn page_titled(&self, title: &str) -> Option<(String, Vec<Value>)> {
            let w = self.0.lock().unwrap();
            w.pages.iter().find(|(_, p)| p.1 == title).map(|(id, p)| (id.clone(), p.2.clone()))
        }
    }

    impl NotionApi for FakeNotion {
        fn call(&self, method: &'static str, path: &str, body: Option<&Value>) -> Result<Value> {
            let mut w = self.0.lock().unwrap();
            w.calls.push(format!("{method} {path}"));
            let seg: Vec<&str> = path.split(['/', '?']).collect();
            match (method, seg.as_slice()) {
                ("POST", ["", "v1", "pages"]) => {
                    let b = body.unwrap();
                    let parent = b["parent"]["page_id"].as_str().unwrap_or("").to_string();
                    if parent != "0123456789abcdef0123456789abcdef" && !w.pages.contains_key(&parent) {
                        bail!("Notion: not found (parent)");
                    }
                    w.next += 1;
                    let id = format!("page{}", w.next);
                    let title: String = b["properties"]["title"]["title"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|t| t["text"]["content"].as_str())
                        .collect();
                    let blocks = b["children"].as_array().cloned().unwrap_or_default();
                    w.pages.insert(id.clone(), (parent, title, blocks));
                    Ok(json!({"id": id, "url": format!("https://www.notion.so/{id}")}))
                }
                ("GET", ["", "v1", "blocks", id, "children", ..]) => {
                    let bl = w.pages.get(*id).map(|p| p.2.clone()).unwrap_or_default();
                    let mut results: Vec<Value> =
                        (0..bl.len()).map(|i| json!({"id": format!("{id}:b{i}"), "type": "paragraph"})).collect();
                    // Its child pages are blocks of it too, as in Notion.
                    results.extend(
                        w.pages.iter().filter(|(_, p)| p.0 == *id).map(|(c, _)| json!({"id": c, "type": "child_page"})),
                    );
                    Ok(json!({"results": results, "has_more": false}))
                }
                ("DELETE", ["", "v1", "blocks", bid]) => {
                    match bid.split_once(":b") {
                        Some((page, _)) => {
                            if let Some(p) = w.pages.get_mut(page) {
                                p.2.pop();
                            }
                        }
                        // A child page block: the page is archived.
                        None => {
                            w.pages.remove(*bid);
                        }
                    }
                    Ok(json!({}))
                }
                ("PATCH", ["", "v1", "blocks", id, "children"]) => {
                    if w.fail_patch > 0 {
                        w.fail_patch -= 1;
                        bail!("Notion: 502 bad gateway");
                    }
                    let add = body.unwrap()["children"].as_array().cloned().unwrap_or_default();
                    let Some(p) = w.pages.get_mut(*id) else { bail!("Notion: not found") };
                    p.2.extend(add);
                    Ok(json!({}))
                }
                ("GET", ["", "v1", "comments", q, ..]) => {
                    let page = q.split('&').find_map(|kv| kv.strip_prefix("block_id=")).unwrap_or("");
                    let from: usize = q
                        .split('&')
                        .find_map(|kv| kv.strip_prefix("start_cursor="))
                        .and_then(|c| c.parse().ok())
                        .unwrap_or(0);
                    let all: Vec<Value> = w
                        .comments
                        .iter()
                        .filter(|c| c.1 == page)
                        .map(|c| json!({"id": c.0, "created_by": {"id": c.2}, "rich_text": [{"plain_text": c.3}]}))
                        .collect();
                    // Two per page, like a long discussion over several pages.
                    let results: Vec<Value> = all.iter().skip(from).take(2).cloned().collect();
                    let more = from + 2 < all.len();
                    Ok(
                        json!({"results": results, "has_more": more, "next_cursor": more.then(|| (from + 2).to_string())}),
                    )
                }
                other => bail!("unexpected Notion call {other:?}"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_to_blocks() {
        let md = "# Spec\n\nSome **text** with a [link](https://x.y/z) and `code`.\nSame paragraph.\n\n## Scope\n- one\n- two\n1. first\n\n| A | B |\n|---|---|\n| 1 | 2 |\n\n```rust\nfn x() {}\n```\n<div>raw</div>\n";
        let b = blocks(md);
        let kinds: Vec<&str> = b.iter().map(|x| x["type"].as_str().unwrap()).collect();
        assert_eq!(
            kinds,
            [
                "heading_1",
                "paragraph",
                "heading_2",
                "bulleted_list_item",
                "bulleted_list_item",
                "numbered_list_item",
                "table",
                "code",
                "code"
            ]
        );
        let para = &b[1]["paragraph"]["rich_text"];
        assert_eq!(para[1]["text"]["link"]["url"], "https://x.y/z");
        assert_eq!(para[3]["annotations"]["code"], true);
        assert!(para[4]["text"]["content"].as_str().unwrap().contains("Same paragraph"));
        assert_eq!(b[6]["table"]["children"].as_array().unwrap().len(), 2, "the separator row is dropped");
        assert_eq!(b[7]["code"]["language"], "rust");
        assert_eq!(b[8]["code"]["language"], "plain text");
        let long = "x".repeat(4500);
        assert_eq!(rich_text(&long).len(), 3, "2000 chars per text");
        assert_eq!(
            page_id("https://www.notion.so/me/Docs-0123456789abcdef0123456789abcdef").as_deref(),
            Some("0123456789abcdef0123456789abcdef")
        );
        assert_eq!(
            page_id("01234567-89ab-cdef-0123-456789abcdef").as_deref(),
            Some("0123456789abcdef0123456789abcdef")
        );
        assert_eq!(page_id("nope"), None);
    }

    /// FR-505 through the runtime: publish from the integration branch,
    /// publish again (same pages, refreshed), comments come back once.
    #[test]
    fn publish_refresh_and_comments_to_the_inbox() {
        let d = tempfile::Builder::new().prefix("orchrt").tempdir_in("/tmp").unwrap();
        let fake = fake::FakeNotion::default();
        let mut cfg = crate::runtime::tests::test_cfg(d.path(), true);
        let f2 = fake.clone();
        cfg.notion = Some(NotionFactory(Arc::new(move || Box::new(f2.clone()))));
        let rt = Runtime::new(cfg).unwrap();
        let wt = d.path().join("integration");
        let docs = wt.join(".orchestrator/features/login");
        std::fs::create_dir_all(&docs).unwrap();
        std::fs::write(docs.join("spec.md"), "# Login\n\nThe spec.\n").unwrap();
        std::fs::write(docs.join("decisions.md"), "- D1 accepted\n").unwrap();
        let project = orch_store::Project {
            id: "p".into(),
            name: "shop".into(),
            path: d.path().into(),
            default_branch: "main".into(),
            remotes: vec![],
            worktree_root: d.path().join("w"),
            task_backend: json!({"type": "local"}),
            track_orchestrator_dir: true,
            settings: json!({}),
            created_at_ms: 1,
            last_active_at_ms: 1,
            last_fetch_at_ms: None,
            last_fetch_error: None,
            archived: false,
        };
        rt.db.upsert_project(&project).unwrap();
        rt.db
            .upsert_feature(&orch_store::Feature {
                id: "f1".into(),
                project_id: "p".into(),
                slug: "login".into(),
                title: "Login".into(),
                brief: String::new(),
                base: "main".into(),
                integration_worktree: Some(wt.clone()),
                allocation: None,
                created_at_ms: 1,
                archived: false,
            })
            .unwrap();
        let e = rt.notion_publish("f1").unwrap_err().to_string();
        assert!(e.contains("parent page"), "{e}");
        let settings = orch_store::ProjectSettings {
            notion_parent_page: Some("https://www.notion.so/me/Docs-0123456789abcdef0123456789abcdef".into()),
            ..Default::default()
        };
        rt.db.set_project_settings("p", &settings, &orch_store::TaskBackendConfig::Local, true).unwrap();

        let v = rt.notion_publish("f1").unwrap();
        assert_eq!(v["docs"], json!(["spec.md", "decisions.md"]));
        let url = v["page_url"].as_str().unwrap().to_string();
        let (spec_id, spec) = fake.page_titled("spec.md").unwrap();
        assert_eq!(spec[0]["type"], "heading_1");
        let pages = fake.0.lock().unwrap().pages.len();
        // Publish again after an edit: the same pages, their blocks replaced.
        std::fs::write(docs.join("spec.md"), "# Login\n\nThe spec, v2.\n\n## More\n").unwrap();
        rt.notion_publish("f1").unwrap();
        assert_eq!(fake.0.lock().unwrap().pages.len(), pages, "refreshed, not duplicated");
        let (_, spec) = fake.page_titled("spec.md").unwrap();
        assert_eq!(spec.len(), 3, "{spec:?}");

        // A comment comes back to the Inbox once.
        fake.comment(&spec_id, "Ada", "Should login support SSO?");
        rt.notion_poll("f1").unwrap();
        rt.notion_poll("f1").unwrap();
        let items: Vec<Value> =
            rt.store.read(|s| s.inbox.iter().filter(|i| i.kind == "notion").map(|i| i.payload.clone()).collect());
        assert_eq!(items.len(), 1);
        assert_eq!(
            (items[0]["kind"].as_str(), items[0]["source"].as_str(), items[0]["author"].as_str()),
            (Some("question"), Some("notion"), Some("Ada"))
        );
        assert_eq!(
            (items[0]["text"].as_str(), items[0]["page_url"].as_str()),
            (Some("Should login support SSO?"), Some(url.as_str()))
        );
        let st = rt.notion_status("f1").unwrap();
        assert_eq!((st["comments_seen"].as_u64(), st["error"].clone()), (Some(1), Value::Null));
    }

    /// Review fixes (T-11): URL forms, table and run limits, 429 retries.
    #[test]
    fn page_ids_limits_and_retries() {
        let id = Some("0123456789abcdef0123456789abcdef".to_string());
        assert_eq!(page_id("https://www.notion.so/me/Docs-0123456789abcdef0123456789abcdef?pvs=4"), id);
        assert_eq!(page_id("https://www.notion.so/Docs-0123456789abcdef0123456789abcdef?source=copy_link#x"), id);
        let mut md = String::from("| A | B |\n|---|---|\n");
        for i in 0..250 {
            md.push_str(&format!("| {i} | x |\n"));
        }
        let b = blocks(&md);
        assert_eq!(b.len(), 3, "250 rows: 3 tables");
        for t in &b {
            let rows = t["table"]["children"].as_array().unwrap();
            assert!(rows.len() <= 100 && rows[0]["table_row"]["cells"][0][0]["text"]["content"] == "A", "header kept");
        }
        let links: String = (0..300).map(|i| format!("[l{i}](https://x.y/{i}) ")).collect();
        let p = &blocks(&links)[0];
        assert!(p["paragraph"]["rich_text"].as_array().unwrap().len() <= 100);

        #[derive(Debug, Default)]
        struct Flaky(std::sync::Mutex<u32>);
        impl Net for Flaky {
            fn http(&self, _: &HttpRequest) -> Result<crate::net::HttpResponse> {
                let mut n = self.0.lock().unwrap();
                *n += 1;
                let status = if *n <= 2 { 429 } else { 200 };
                Ok(crate::net::HttpResponse {
                    status,
                    headers: vec![("retry-after".into(), "0.5".into())],
                    body: "{}".into(),
                    timed_out: false,
                })
            }
            fn gh_auth_status(&self) -> Result<String> {
                bail!("no")
            }
        }
        let net = Flaky::default();
        let req = HttpRequest {
            method: "DELETE",
            url: "https://api.notion.com/v1/blocks/x".into(),
            headers: vec![],
            body: None,
            timeout: Duration::from_secs(1),
            max_body: crate::net::MAX_BODY,
        };
        let waits = std::sync::Mutex::new(vec![]);
        let r = http_with_retry(&net, &req, |d| waits.lock().unwrap().push(d)).unwrap();
        assert_eq!((r.status, *net.0.lock().unwrap()), (200, 3));
        assert_eq!(waits.lock().unwrap().as_slice(), [Duration::from_millis(500); 2]);
    }

    /// Review fixes (T-11): every page of comments is read; a failed append
    /// after a page was made never makes a second page; a publish and polls
    /// at the same time make each comment's Inbox item once.
    #[test]
    fn comments_pages_failed_appends_and_concurrency() {
        let d = tempfile::Builder::new().prefix("orchrt").tempdir_in("/tmp").unwrap();
        let fake = fake::FakeNotion::default();
        let mut cfg = crate::runtime::tests::test_cfg(d.path(), true);
        let f2 = fake.clone();
        cfg.notion = Some(NotionFactory(Arc::new(move || Box::new(f2.clone()))));
        let rt = Runtime::new(cfg).unwrap();
        let wt = d.path().join("integration");
        let docs = wt.join(".orchestrator/features/login");
        std::fs::create_dir_all(&docs).unwrap();
        // Over 100 blocks: the page is made with 100, the rest appended.
        let long: String = (0..150).map(|i| format!("- item {i}\n")).collect();
        std::fs::write(docs.join("spec.md"), long).unwrap();
        setup_feature(&rt, d.path(), &wt);
        fake.0.lock().unwrap().fail_patch = 1;
        assert!(rt.notion_publish("f1").is_err(), "the append failed");
        let pages = fake.0.lock().unwrap().pages.len();
        rt.notion_publish("f1").unwrap();
        assert_eq!(fake.0.lock().unwrap().pages.len(), pages, "no second page for the doc");
        let (spec_id, spec) = fake.page_titled("spec.md").unwrap();
        assert_eq!(spec.len(), 150);

        for i in 0..5 {
            fake.comment(&spec_id, "Ada", &format!("c{i}"));
        }
        std::thread::scope(|s| {
            for k in 0..4 {
                let rt = rt.clone();
                s.spawn(move || {
                    if k == 0 {
                        rt.notion_publish("f1").unwrap();
                    } else {
                        rt.notion_poll("f1").unwrap();
                    }
                });
            }
        });
        rt.notion_poll("f1").unwrap();
        let n = rt.store.read(|s| s.inbox.iter().filter(|i| i.kind == "notion").count());
        assert_eq!(n, 5, "every comment of every page, once");
    }

    fn setup_feature(rt: &Runtime, d: &std::path::Path, wt: &std::path::Path) {
        let project = orch_store::Project {
            id: "p".into(),
            name: "shop".into(),
            path: d.into(),
            default_branch: "main".into(),
            remotes: vec![],
            worktree_root: d.join("w"),
            task_backend: json!({"type": "local"}),
            track_orchestrator_dir: true,
            settings: json!({}),
            created_at_ms: 1,
            last_active_at_ms: 1,
            last_fetch_at_ms: None,
            last_fetch_error: None,
            archived: false,
        };
        rt.db.upsert_project(&project).unwrap();
        rt.db
            .upsert_feature(&orch_store::Feature {
                id: "f1".into(),
                project_id: "p".into(),
                slug: "login".into(),
                title: "Login".into(),
                brief: String::new(),
                base: "main".into(),
                integration_worktree: Some(wt.into()),
                allocation: None,
                created_at_ms: 1,
                archived: false,
            })
            .unwrap();
        let settings = orch_store::ProjectSettings {
            notion_parent_page: Some("0123456789abcdef0123456789abcdef".into()),
            ..Default::default()
        };
        rt.db.set_project_settings("p", &settings, &orch_store::TaskBackendConfig::Local, true).unwrap();
    }
}
