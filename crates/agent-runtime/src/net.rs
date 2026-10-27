//! The core's few network calls (FR-604 probes, FR-1203 token tests): HTTP
//! through `/usr/bin/curl` (the system's TLS and proxy settings, no TLS stack
//! in the app) and `gh auth status`. Headers go to curl on **stdin**
//! (`-H @-`), so a token is never on a command line (NFR-3).

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result, bail};

/// One request. Header values may carry a secret: they are only written to
/// curl's stdin, never logged.
#[derive(Clone)]
pub struct HttpRequest {
    pub method: &'static str,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
    pub timeout: Duration,
}

impl std::fmt::Debug for HttpRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Header names only.
        let names: Vec<&str> = self.headers.iter().map(|(k, _)| k.as_str()).collect();
        f.debug_struct("HttpRequest")
            .field("method", &self.method)
            .field("url", &self.url)
            .field("headers", &names)
            .finish()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct HttpResponse {
    pub status: u16,
    /// Lower-cased names.
    pub headers: Vec<(String, String)>,
    pub body: String,
    /// The deadline cut the transfer (an SSE stream stays open).
    pub timed_out: bool,
}

impl HttpResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

/// Network access, replaceable in tests.
pub trait Net: Send + Sync + std::fmt::Debug {
    fn http(&self, req: &HttpRequest) -> Result<HttpResponse>;
    /// `gh auth status`: its output when logged in, else the error text.
    fn gh_auth_status(&self) -> Result<String>;
}

/// The real one.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemNet;

/// Most bytes of a response body kept.
const MAX_BODY: usize = 256 * 1024;

impl Net for SystemNet {
    fn http(&self, req: &HttpRequest) -> Result<HttpResponse> {
        let mut stdin = String::new();
        for (k, v) in &req.headers {
            if k.contains(['\r', '\n', ':']) || v.contains(['\r', '\n']) {
                bail!("invalid header {k}");
            }
            stdin.push_str(&format!("{k}: {v}\n"));
        }
        let secs = req.timeout.as_secs().max(1).to_string();
        let mut cmd = Command::new("/usr/bin/curl");
        cmd.args(["-sS", "-N", "--max-time", &secs, "--max-redirs", "0", "-D", "-", "-o", "-", "-X", req.method]);
        cmd.args(["-H", "@-"]);
        // The body (task text, whole docs) never goes on the command line:
        // a 0600 file, removed when curl is done.
        let body_file = req.body.as_deref().map(BodyFile::new).transpose()?;
        if let Some(f) = &body_file {
            cmd.arg("--data-binary").arg(format!("@{}", f.0.display()));
        }
        cmd.arg("--").arg(&req.url);
        let mut child =
            cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().context("starting curl")?;
        if let Some(mut i) = child.stdin.take() {
            i.write_all(stdin.as_bytes())?;
        }
        let out = child.wait_with_output()?;
        // 28: the deadline. Whatever came before it still counts (an SSE stream).
        let timed_out = out.status.code() == Some(28);
        if !out.status.success() && !timed_out {
            bail!("{}", String::from_utf8_lossy(&out.stderr).trim());
        }
        let mut r = parse_curl_output(&out.stdout);
        r.timed_out = timed_out;
        if r.status == 0 {
            bail!("no answer from {}", host_of(&req.url));
        }
        Ok(r)
    }

    fn gh_auth_status(&self) -> Result<String> {
        let mut cmd = Command::new("gh");
        cmd.args(["auth", "status"]).env("GH_PROMPT_DISABLED", "1").env("NO_COLOR", "1");
        let out = orch_git::git::run_with_deadline(cmd, Duration::from_secs(15))?;
        // Older gh prints the status on stderr.
        let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        if !out.status.success() {
            bail!("{}", text.trim());
        }
        Ok(text)
    }
}

/// A request body in a private temp file, deleted on drop.
struct BodyFile(std::path::PathBuf);

impl BodyFile {
    fn new(body: &str) -> Result<BodyFile> {
        use std::os::unix::fs::OpenOptionsExt;
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("orch-body-{}-{n}-{}", std::process::id(), orch_protocol::now_ms()));
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .with_context(|| format!("creating {}", path.display()))?;
        let file = BodyFile(path);
        f.write_all(body.as_bytes())?;
        Ok(file)
    }
}

impl Drop for BodyFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// `-D - -o -`: header blocks (a proxy's `200 Connection Established` comes
/// first), then the body.
pub fn parse_curl_output(raw: &[u8]) -> HttpResponse {
    let text = String::from_utf8_lossy(&raw[..raw.len().min(MAX_BODY)]).into_owned();
    let mut rest = text.as_str();
    let mut r = HttpResponse::default();
    while rest.starts_with("HTTP/") {
        let (block, after) = match rest.find("\r\n\r\n") {
            Some(i) => (&rest[..i], &rest[i + 4..]),
            None => (rest, ""),
        };
        let mut lines = block.lines();
        r.status = lines.next().and_then(|l| l.split_whitespace().nth(1)).and_then(|c| c.parse().ok()).unwrap_or(0);
        r.headers = lines
            .filter_map(|l| l.split_once(':'))
            .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
            .collect();
        rest = after;
    }
    r.body = rest.to_string();
    r
}

pub fn host_of(url: &str) -> &str {
    let s = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    s.split(['/', '?', '#']).next().unwrap_or(s).rsplit('@').next().unwrap_or(s)
}

#[cfg(test)]
pub(crate) mod fake {
    use std::sync::Mutex;

    use super::*;

    /// Answers by URL; records the requests (with their headers, for the tests).
    #[derive(Debug, Default)]
    pub struct FakeNet {
        pub routes: Mutex<Vec<(String, HttpResponse)>>,
        pub seen: Mutex<Vec<HttpRequest>>,
        pub gh: Mutex<Option<Result<String, String>>>,
    }

    impl FakeNet {
        pub fn route(&self, url: &str, status: u16, headers: &[(&str, &str)], body: &str) {
            self.routes.lock().unwrap().retain(|(u, _)| u != url);
            self.routes.lock().unwrap().push((
                url.into(),
                HttpResponse {
                    status,
                    headers: headers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
                    body: body.into(),
                    timed_out: false,
                },
            ));
        }
    }

    impl Net for FakeNet {
        fn http(&self, req: &HttpRequest) -> Result<HttpResponse> {
            self.seen.lock().unwrap().push(req.clone());
            match self.routes.lock().unwrap().iter().find(|(u, _)| *u == req.url) {
                Some((_, r)) => Ok(r.clone()),
                None => bail!("could not resolve host {}", host_of(&req.url)),
            }
        }
        fn gh_auth_status(&self) -> Result<String> {
            match self.gh.lock().unwrap().clone() {
                Some(Ok(s)) => Ok(s),
                Some(Err(e)) => bail!("{e}"),
                None => bail!("gh: not installed"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curl_output_with_a_proxy_block() {
        let raw = b"HTTP/1.1 200 Connection Established\r\n\r\nHTTP/2 401 \r\nwww-authenticate: Bearer realm=\"OAuth\"\r\nContent-Type: application/json\r\n\r\n{\"error\":1}";
        let r = parse_curl_output(raw);
        assert_eq!(r.status, 401);
        assert_eq!(r.header("www-authenticate"), Some("Bearer realm=\"OAuth\""));
        assert_eq!(r.header("content-type"), Some("application/json"));
        assert_eq!(r.body, "{\"error\":1}");
        assert_eq!(parse_curl_output(b"").status, 0);
    }

    #[test]
    fn hosts_and_redacted_debug() {
        assert_eq!(host_of("https://api.linear.app/graphql"), "api.linear.app");
        assert_eq!(host_of("https://u:p@x.example:8443/mcp?t=1"), "x.example:8443");
        let r = HttpRequest {
            method: "GET",
            url: "https://x".into(),
            headers: vec![("Authorization".into(), "Bearer s3cret".into())],
            body: None,
            timeout: Duration::from_secs(1),
        };
        let d = format!("{r:?}");
        assert!(d.contains("Authorization") && !d.contains("s3cret"), "{d}");
    }

    #[test]
    fn a_body_goes_through_a_private_file_removed_after() {
        use std::os::unix::fs::PermissionsExt;
        let f = BodyFile::new("{\"query\": \"secret task text\"}").unwrap();
        let p = f.0.clone();
        assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "{\"query\": \"secret task text\"}");
        drop(f);
        assert!(!p.exists());
    }

    #[test]
    fn header_injection_is_refused() {
        let r = HttpRequest {
            method: "GET",
            url: "http://127.0.0.1:9/".into(),
            headers: vec![("X".into(), "a\nHost: evil".into())],
            body: None,
            timeout: Duration::from_secs(1),
        };
        assert!(SystemNet.http(&r).unwrap_err().to_string().contains("invalid header"));
    }
}
